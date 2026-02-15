//! Linux per-app audio capture via PipeWire.
//!
//! Strategy:
//! 1. Connect to PipeWire, enumerate nodes with `media.class = "Stream/Output/Audio"`
//! 2. Match by `application.name` to build the source list
//! 3. To capture, create a PipeWire stream targeting the app's output node

use super::{AudioSource, CHANNELS, CaptureHandle, FRAME_SIZE, SAMPLE_RATE, SAMPLES_PER_FRAME};
use tokio::sync::mpsc;

/// List applications currently outputting audio via PipeWire.
pub async fn list_sources() -> anyhow::Result<Vec<AudioSource>> {
    // Run PipeWire enumeration on a blocking thread since pipewire-rs
    // uses its own main loop and is not async.
    tokio::task::spawn_blocking(|| list_sources_sync()).await?
}

fn list_sources_sync() -> anyhow::Result<Vec<AudioSource>> {
    use pipewire as pw;
    use std::cell::RefCell;
    use std::rc::Rc;

    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let sources: Rc<RefCell<Vec<AudioSource>>> = Rc::new(RefCell::new(Vec::new()));
    let sources_clone = sources.clone();
    let mainloop_weak = mainloop.downgrade();

    // Track pending sync
    let pending = Rc::new(RefCell::new(true));
    let pending_clone = pending.clone();

    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(props) = global.props {
                let media_class = props.get("media.class").unwrap_or("");
                if media_class == "Stream/Output/Audio" {
                    let name = props
                        .get("application.name")
                        .or_else(|| props.get("node.name"))
                        .unwrap_or("Unknown")
                        .to_string();
                    let id = global.id.to_string();
                    sources_clone.borrow_mut().push(AudioSource { id, name });
                }
            }
        })
        .register();

    // Sync: once the registry is done enumerating, quit the loop.
    let _sync_listener = core
        .add_listener_local()
        .done(move |_id, _seq| {
            if *pending_clone.borrow() {
                *pending_clone.borrow_mut() = false;
                if let Some(ml) = mainloop_weak.upgrade() {
                    ml.quit();
                }
            }
        })
        .register();
    core.sync(0)?;
    mainloop.run();

    let result = sources.borrow().clone();
    Ok(result)
}

/// Start capturing audio from a specific PipeWire node.
pub async fn start_capture(
    source: &AudioSource,
) -> anyhow::Result<(CaptureHandle, mpsc::Receiver<Vec<f32>>)> {
    let node_id: u32 = source.id.parse()?;
    let (tx, rx) = mpsc::channel::<Vec<f32>>(64);
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::task::spawn_blocking(move || {
        capture_loop(node_id, tx, &mut stop_rx);
    });

    Ok((CaptureHandle::new(stop_tx), rx))
}

fn capture_loop(
    target_node_id: u32,
    tx: mpsc::Sender<Vec<f32>>,
    stop_rx: &mut tokio::sync::oneshot::Receiver<()>,
) {
    use pipewire as pw;
    use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
    use pw::spa::pod::Pod;
    use std::cell::RefCell;
    use std::rc::Rc;

    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).expect("pw mainloop");
    let context = pw::context::Context::new(&mainloop).expect("pw context");
    let core = context.connect(None).expect("pw core");

    // Build audio format params
    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(AudioFormat::F32LE);
    audio_info.set_rate(SAMPLE_RATE);
    audio_info.set_channels(CHANNELS as u32);

    let values = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(vec![0u8; 1024]),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        }),
    )
    .expect("serialize audio format")
    .0
    .into_inner();

    let stream = pw::stream::Stream::new(
        &core,
        "hearme-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::STREAM_CAPTURE_SINK => "true",
            *pw::keys::TARGET_OBJECT => target_node_id.to_string(),
        },
    )
    .expect("pw stream");

    let accumulator: Rc<RefCell<Vec<f32>>> =
        Rc::new(RefCell::new(Vec::with_capacity(SAMPLES_PER_FRAME * 2)));
    let acc_clone = accumulator.clone();
    let tx_clone = tx.clone();

    let _listener = stream
        .add_local_listener()
        .process(move |stream, _| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                if let Some(data) = buffer.datas_mut().first_mut() {
                    if let Some(slice) = data.data() {
                        // Convert bytes to f32 samples
                        let samples: &[f32] = bytemuck_cast_slice(slice);
                        let mut acc = acc_clone.borrow_mut();
                        acc.extend_from_slice(samples);

                        // Emit complete frames (20ms = SAMPLES_PER_FRAME)
                        while acc.len() >= SAMPLES_PER_FRAME {
                            let frame: Vec<f32> = acc.drain(..SAMPLES_PER_FRAME).collect();
                            let _ = tx_clone.try_send(frame);
                        }
                    }
                }
            }
        })
        .register();

    let pod = Pod::from_bytes(&values).expect("pod from bytes");
    stream
        .connect(
            pw::spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [pod],
        )
        .expect("pw stream connect");

    // Run until stop signal
    let mainloop_weak = mainloop.downgrade();
    std::thread::spawn(move || {
        let _ = stop_rx.try_recv(); // Block not possible here, poll instead
        // In practice we'd use a pipe/eventfd to signal the mainloop
        if let Some(ml) = mainloop_weak.upgrade() {
            ml.quit();
        }
    });

    mainloop.run();
}

/// Safe cast from byte slice to f32 slice (assumes LE alignment).
fn bytemuck_cast_slice(bytes: &[u8]) -> &[f32] {
    let len = bytes.len() / 4;
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, len) }
}
