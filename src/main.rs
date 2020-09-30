use bytes::buf::BufMutExt;
use cairo::{Context, Format, ImageSurface};

use bytes::{Bytes, BytesMut};
use futures::ready;
use futures::Stream;
use futures::StreamExt;
use piet::{Color, RenderContext, Text, TextLayout, TextLayoutBuilder};
use piet_cairo::CairoRenderContext;
use pin_project::pin_project;
use std::io::Write;
use std::sync::Arc;
use std::task::Poll;
use tokio::time::interval;
use warp::Filter;

const WIDTH: i32 = 600;
const HEIGHT: i32 = 80;
const HIDPI: f64 = 2.0;

const BOUNDARY: &'static str = "1S6F7kYNCc3kHei4MESUV0MDgIHFN53E";
const CONTENT_TYPE: &'static str =
    "multipart/x-mixed-replace; boundary=1S6F7kYNCc3kHei4MESUV0MDgIHFN53E";

fn render(out: &mut impl std::io::Write, count: usize) {
    let surface = ImageSurface::create(Format::Rgb24, WIDTH, HEIGHT).unwrap();

    let mut cr = Context::new(&surface);
    cr.scale(HIDPI, HIDPI);
    let mut piet_ctx = CairoRenderContext::new(&mut cr);

    piet_ctx.clear(Color::rgb(0xff, 0, 0xff));

    let now = chrono::Utc::now().to_rfc2822();

    let text = piet_ctx.text();
    let font = piet::FontFamily::SYSTEM_UI;

    let time_layout = text
        .new_text_layout(now)
        .font(font.clone(), 16.)
        .max_width(WIDTH as f64)
        .text_color(piet::Color::WHITE)
        .build()
        .unwrap();

    let time_y_pos = (((HEIGHT as f64 / 2.0) - time_layout.size().height * 2.0) / 4.0).max(0.0);

    let count_layout = text
        .new_text_layout(format!("current viewers: {}", count))
        .font(font, 16.)
        .max_width(WIDTH as f64)
        .text_color(piet::Color::WHITE)
        .build()
        .unwrap();

    let count_y_pos = ((HEIGHT as f64 / 4.0) - ((HEIGHT as f64 / 2.0) - count_layout.size().height * 2.0) / 4.0).max(0.0);

    piet_ctx.draw_text(&time_layout, (8., time_y_pos));
    piet_ctx.draw_text(&count_layout, (8., count_y_pos));

    piet_ctx.finish().unwrap();
    surface.flush();

    surface.write_to_png(out).expect("Error writing image file");
}

async fn renderer(chan: tokio::sync::broadcast::Sender<Bytes>, count: Arc<()>) {
    let mut bytes = BytesMut::new().writer();
    let mut interval = interval(tokio::time::Duration::from_secs(1));

    loop {
        interval.tick().await;

        render(&mut bytes, Arc::strong_count(&count) / 2 - 1);

        let image_bytes = bytes.get_mut().split();

        let image = image::load_from_memory(&image_bytes).unwrap();
        image
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .unwrap();
        let image_data = bytes.get_mut().split();

        write!(
            bytes,
            "------{}\r\nContent-Type: image/jpeg\r\nContent-length: {}\r\n\r\n",
            BOUNDARY,
            image_data.len()
        )
        .unwrap();

        let mut data = bytes.get_mut().split();

        data.unsplit(image_data);
        data.extend_from_slice(b"\r\n");

        chan.send(data.freeze()).unwrap();
    }
}

#[pin_project]
struct AliveCounter<S> {
    counter: Arc<()>,
    #[pin]
    stream: S,
}

impl<S> AliveCounter<S> {
    fn new(counter: Arc<()>, stream: S) -> Self { Self { counter, stream } }
}


impl<S: Stream> Stream for AliveCounter<S> {
    type Item = <S as Stream>::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.project();
        let res = ready!(this.stream.poll_next(cx));
        Poll::Ready(res)
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let (watch_in, _watch_out) = tokio::sync::broadcast::channel(1);
    let sender = watch_in.clone();

    let live_counter = Arc::new(());
    tokio::spawn(renderer(watch_in, live_counter.clone()));

    let time = warp::path!("time").map(move || {
        let s = sender
            .subscribe()
            .into_stream()
            .filter(|r| futures::future::ready(r.is_ok()));

        let s = AliveCounter::new(live_counter.clone(), s);

        warp::http::Response::builder()
            .header("Age", "0")
            .header("Cache-Control", "no-cache, private")
            .header("Pragma", "no-cache")
            .header("Content-Type", CONTENT_TYPE)
            .body(hyper::Body::wrap_stream(s))
            .unwrap()
    });

    warp::serve(time).run(([127, 0, 0, 1], 3030)).await;
}
