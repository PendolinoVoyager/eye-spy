use std::time::Duration;

use h264_stream::{init_client_streams, RGB_FRAME_BUFFER};
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::WindowCanvas;
use sdl2::sys::exit;
use sdl2::EventPump;

pub(crate) mod h264_stream;

fn read_events(event_pump: &mut EventPump) {
    use sdl2::event::Event;
    for event in event_pump.poll_iter() {
        match event {
            Event::Quit { .. } => unsafe { exit(0) },
            _ => (),
        }
        continue;
    }
}

fn render(canvas: &mut WindowCanvas, color: Color) {
    canvas.set_draw_color(color);
    canvas.clear();
    canvas.present();
}

fn main() {
    let sdl = sdl2::init().unwrap();
    let video_subsystem = sdl.video().unwrap();
    let window = video_subsystem
        .window("Eye Spy", 1920, 1080)
        .resizable()
        .position_centered()
        .maximized()
        .build()
        .unwrap();
    let mut canvas = window.into_canvas().build().unwrap();

    let mut event_pump = sdl.event_pump().unwrap();

    init_client_streams();
    let texture_creator = canvas.texture_creator();

    // Create a texture to store RGB data
    let mut texture = texture_creator
        .create_texture_streaming(
            sdl2::pixels::PixelFormatEnum::RGB24,
            h264_stream::WIDTH as u32,
            h264_stream::HEIGHT as u32,
        )
        .unwrap();

    loop {
        read_events(&mut event_pump);
        canvas.set_draw_color(Color::RGB(0, 255, 255));
        canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 0, 0));
        canvas.clear();

        texture
            .update(
                None,
                &RGB_FRAME_BUFFER.lock().unwrap()[..],
                h264_stream::WIDTH * 3,
            )
            .unwrap();
        // Copy the texture to the canvas and render it
        canvas
            .copy(
                &texture,
                None,
                Some(Rect::new(
                    0,
                    0,
                    h264_stream::WIDTH as u32,
                    h264_stream::HEIGHT as u32,
                )),
            )
            .unwrap();
        canvas.present();
        std::thread::sleep(Duration::from_micros(16700));
    }
}
