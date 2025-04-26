use clap::Parser;
use ffmpeg_next::{self as ffmpeg};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use rascii_art::{render_to, RenderOptions};
use rusttype::{Font, Scale};
use std::{error::Error, path::Path, sync::Mutex};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short = 'i', long)]
    input_video_path: String,
    #[arg(short = 'o', long)]
    output_video_path: String,
    #[arg(short = 'f', long)]
    output_frames_path: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let input_video_path = &args.input_video_path;
    let output_video_path = &args.output_video_path;
    let output_frames_path = &args.output_frames_path;

    process_video_to_ascii(input_video_path, output_frames_path)?;

    frames_to_video(output_frames_path, output_video_path)?;

    println!("Video processing completed successfully.");
    Ok(())
}

fn process_video_to_ascii(input_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    ffmpeg::init()?;

    std::fs::create_dir_all(output_path)?;

    let mut input_context = ffmpeg::format::input(&input_path)?;

    // 最適なビデオストリームを見つける
    let input_stream = input_context
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("cloud not found video stream")?;

    let stream_index = input_stream.index();

    // デコーダを生成
    let context_decoder = ffmpeg::codec::Context::from_parameters(input_stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;

    // フレームの情報を取得
    let width = decoder.width();
    let height = decoder.height();

    println!("video size: {}x{}", width, height);

    // フレームバッファを作成
    let mut frame = ffmpeg::frame::Video::empty();
    let mut rgb_frame = ffmpeg::frame::Video::empty();
    let mut frame_count = 0;

    // スケーラーを設定（フレームをRGBに変換するため）
    let mut scaler = ffmpeg::software::scaling::context::Context::get(
        decoder.format(),
        width,
        height,
        ffmpeg::format::pixel::Pixel::RGB24,
        width,
        height,
        ffmpeg::software::scaling::flag::Flags::BILINEAR,
    )?;

    for (stream, packet) in input_context.packets() {
        if stream.index() == stream_index {
            decoder.send_packet(&packet)?;

            while decoder.receive_frame(&mut frame).is_ok() {
                // フレームをRGBに変換
                scaler.run(&frame, &mut rgb_frame)?;

                let temp_frame_path = format!("{}/temp_frame.png", output_path);
                save_frame_as_image(&rgb_frame, &temp_frame_path)?;

                let ascii_text = image_to_text(&temp_frame_path)?;

                let output_frame_path = format!("{}/frame_{:04}.png", output_path, frame_count);
                save_text_as_image(&ascii_text, &output_frame_path)?;

                println!("Proceed frame No.{} : {}", frame_count, output_frame_path);
                frame_count += 1;
            }
        }
    }

    // Process remaining frames
    decoder.send_eof()?;
    while decoder.receive_frame(&mut frame).is_ok() {
        scaler.run(&frame, &mut rgb_frame)?;

        // Save frame as temporary image
        let temp_frame_path = format!("{}/temp_frame.png", output_path);
        save_frame_as_image(&rgb_frame, &temp_frame_path)?;

        let ascii_text = image_to_text(&temp_frame_path)?;

        let output_frame_path = format!("{}/frame_{:04}.png", output_path, frame_count);
        save_text_as_image(&ascii_text, &output_frame_path)?;

        println!("Proceed frame No.{} : {}", frame_count, output_frame_path);
        frame_count += 1;
    }

    println!("Proceed {} frames", frame_count);
    Ok(())
}

// フレームをRGBA画像として保存
fn save_frame_as_image(
    frame: &ffmpeg::frame::Video,
    output_path: &str,
) -> Result<(), Box<dyn Error>> {
    let width = frame.width();
    let height = frame.height();
    let data = frame.data(0);
    let linesize = frame.stride(0);

    // RGB データから ImageBuffer を作成
    let mut img = RgbaImage::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let i = (y as usize * linesize) + (x as usize * 3);
            if i + 2 < data.len() {
                let r = data[i];
                let g = data[i + 1];
                let b = data[i + 2];
                img.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
    }

    // 画像を保存
    img.save(Path::new(output_path))?;
    Ok(())
}

fn image_to_text(image_path: &str) -> Result<String, Box<dyn Error>> {
    let mut buffer = String::new();

    render_to(
        image_path,
        &mut buffer,
        &RenderOptions::new()
            .width(100) // ASCIIアートの幅を調整
            .charset(&[".", ",", "-", "*", "£", "$", "#"]),
    )?;

    Ok(buffer)
}

static IS_INITIAL_FLAG: Mutex<bool> = Mutex::new(true);
static BASE_ROW_CHAR_COUNT: Mutex<usize> = Mutex::new(0);
static BASE_LINE_COUNT: Mutex<usize> = Mutex::new(0);

fn save_text_as_image(text: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    let font_size = 10.0; // フォントサイズを小さくして多くの文字を表示

    let max_line_length = text.lines().map(|line| line.len()).max().unwrap_or(1);
    let lines_count = text.lines().count();
    if IS_INITIAL_FLAG.lock().unwrap().clone() {
        *BASE_ROW_CHAR_COUNT.lock().unwrap() = max_line_length;
        *BASE_LINE_COUNT.lock().unwrap() = lines_count;
        *IS_INITIAL_FLAG.lock().unwrap() = false;
    }

    let font_data = include_bytes!("../assets/NeverMindMono-Bold.ttf");
    let font = Font::try_from_bytes(font_data as &[u8]).ok_or("failed to load font file")?;

    let width_scale_factor =
        BASE_ROW_CHAR_COUNT.lock().unwrap().clone() as f32 / max_line_length as f32;
    let height_scale_factor = BASE_LINE_COUNT.lock().unwrap().clone() as f32 / lines_count as f32;
    let opt_font_size = if width_scale_factor > height_scale_factor {
        font_size * width_scale_factor
    } else {
        font_size * height_scale_factor
    };
    // calculate font size by first frame line count and char count on row
    let width = max_line_length as f32 * (font_size * width_scale_factor * 0.4);
    let height = lines_count as f32 * font_size * height_scale_factor;

    let mut img = RgbaImage::new(width as u32, height as u32);

    // Please change color code if you want to change the background color
    for pixel in img.pixels_mut() {
        *pixel = Rgba([0, 0, 0, 255]);
    }

    let scale = Scale::uniform(opt_font_size);

    text.lines().enumerate().for_each(|(i, line)| {
        // 画像にテキストを描画
        draw_text_mut(
            &mut img,
            Rgba([255, 255, 255, 255]), // 白色のテキスト
            0,
            (font_size * i as f32 * 1.2) as i32, // y 座標
            scale,
            &font,
            line,
        );
    });

    img.save(Path::new(output_path))?;

    Ok(())
}

fn frames_to_video(frames_path: &str, output_path: &str) -> Result<(), Box<dyn Error>> {
    let status = std::process::Command::new("ffmpeg")
        .args(&[
            "-framerate",
            "30",
            "-i",
            &format!("{}/frame_%04d.png", frames_path),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            output_path,
            "-y", // overwrite output file
        ])
        .status()?;

    if !status.success() {
        return Err("failed to execute 'ffmpeg' command".into());
    }

    Ok(())
}
