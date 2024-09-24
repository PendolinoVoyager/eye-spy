use std::time::Duration;

use sdl2::pixels::Color;
use sdl2::render::WindowCanvas;
use sdl2::sys::exit;
use sdl2::{event, EventPump, Sdl};
use stream::init_client_streams;

pub mod stream;

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

    let mut i: u8 = 0;
    init_client_streams();
    loop {
        read_events(&mut event_pump);
        canvas.set_draw_color(Color::RGB(0, 255, 255));
        render(&mut canvas, Color::RGB(i, 64, 255 - i));
        canvas.present();
        i = (i + 1) % 255;
        std::thread::sleep(Duration::from_micros(16700));
    }
}
