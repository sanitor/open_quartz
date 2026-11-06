use std::{sync::{Arc, Mutex, mpsc}, time::Duration};
use open_quartz::{
    media::native::{NativeVideoConfig, NativeVideoFrame, NativeVideoSource, NativeVideoSourceKind},
    rendering::{GpuBackend, GpuExecutor},
};
use slint::ComponentHandle;

slint::slint! {
    import { Button } from "std-widgets.slint";
    component MediaView inherits Rectangle {
        in property <image> frame;
        background: #181818;
        Image { width: parent.width; height: parent.height; source: root.frame; image-fit: contain; }
    }
    export component VideoPreview inherits Window {
        title: "VideoToolbox / shared GPU preview";
        width: 800px; height: 540px;
        in property <image> frame;
        in property <string> status: "Opening video…";
        callback transport(string);
        VerticalLayout {
            MediaView { frame: root.frame; vertical-stretch: 1; preferred-height: 400px; }
            Text { text: root.status; wrap: word-wrap; }
            HorizontalLayout {
                Button { text: "Play"; clicked => { root.transport("play"); } }
                Button { text: "Pause"; clicked => { root.transport("pause"); } }
                Button { text: "Stop / rewind"; clicked => { root.transport("stop"); } }
            }
        }
    }
}

#[derive(Default)]
struct Update { texture: Option<wgpu::Texture>, status: Option<String>, changed: bool }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path=std::env::args_os().nth(1).ok_or("usage: cargo run --manifest-path apps/open_quartz_slint/Cargo.toml --example video_preview -- VIDEO")?;
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
        ..Default::default()
    }))?;
    let backend=Arc::new(GpuBackend::from_device(device.clone(),queue.clone()));
    slint::BackendSelector::new().require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Manual{instance,adapter,device,queue}).select()?;
    let window=VideoPreview::new()?;
    let updates=Arc::new(Mutex::new(Update::default()));
    let (tx,rx)=mpsc::channel::<String>();
    let output=updates.clone();
    let worker=std::thread::spawn(move || {
        let result=(||->Result<(),String>{
            let config=NativeVideoConfig{kind:NativeVideoSourceKind::File,source:path.to_string_lossy().into_owned(),looping:true,playback_rate:1.0};
            let mut source=NativeVideoSource::open_preview(config.clone())?;
            let mut executor=GpuExecutor::new(backend);
            let mut playing=false;
            let mut preview=true;
            let mut last_status=String::new();
            loop {
                let command=if playing || preview {rx.recv_timeout(Duration::from_millis(8))}else{rx.recv().map_err(|_|mpsc::RecvTimeoutError::Disconnected)};
                match command {
                    Ok(action) if action=="close"=>break,
                    Ok(action) if action=="play"=>{source.resume()?;playing=true;preview=false;}
                    Ok(action) if action=="pause"=>{source.pause();playing=false;}
                    Ok(action) if action=="stop"=>{source=NativeVideoSource::open_preview(config.clone())?;playing=false;preview=true;}
                    Ok(_)|Err(mpsc::RecvTimeoutError::Timeout)=>{},
                    Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                }
                if playing || preview {
                    let uploaded=source.upload_latest(&mut |frame,w,h|match frame {
                        NativeVideoFrame::Rgba(bytes)=>executor.upload_rgba("video",bytes,w,h).map_err(|e|e.to_string()),
                        #[cfg(target_os="macos")]
                        NativeVideoFrame::VideoToolbox(frame)=>executor.upload_videotoolbox("video",frame).map_err(|e|e.to_string()),
                        #[cfg(windows)]
                        NativeVideoFrame::D3d12(frame)=>executor.upload_d3d12_yuv("video",frame).map_err(|e|e.to_string()),
                    })?;
                    if uploaded {
                        let texture=executor.output_handle("video").ok_or("No decoded GPU output")?;
                        let mut u=output.lock().map_err(|_|"preview update lock poisoned")?;
                        u.texture=Some((*texture.texture).clone());u.changed=true;
                        if preview {preview=false;}
                    }
                }
                let metrics=source.metrics();
                let status=format!("{} · {} · decoded {} / uploaded {} · CPU upload {} bytes{}",source.info().decoder,if playing{"playing"}else{"stopped/paused"},metrics.decoded_frames,metrics.uploaded_frames,metrics.cpu_copy_bytes,source.info().fallback_reason.as_ref().map(|s|format!(" · fallback: {s}")).unwrap_or_default());
                if status!=last_status {let mut u=output.lock().map_err(|_|"preview update lock poisoned")?;u.status=Some(status.clone());last_status=status;}
                if playing && !source.is_active() {playing=false;}
            }
            Ok(())
        })();
        if let Err(error)=result {if let Ok(mut u)=output.lock(){u.status=Some(format!("Video failed: {error}"));}}
    });
    let control=tx.clone();window.on_transport(move|action|{let _=control.send(action.to_string());});
    let weak=window.as_weak();let timer=slint::Timer::default();
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(16),move||{
        if let Some(window)=weak.upgrade(){
            if let Ok(mut u)=updates.lock(){
                if let Some(status)=u.status.take(){window.set_status(status.into());}
                if u.changed {u.changed=false;if let Some(texture)=u.texture.take(){
                    match slint::Image::try_from(texture){Ok(image)=>window.set_frame(image),Err(e)=>window.set_status(format!("Present failed: {e}").into())}
                    window.window().request_redraw();
                }}
            }
        }
    });
    let result=window.run();let _=tx.send("close".into());worker.join().map_err(|_|"video worker panicked")?;result?;
    Ok(())
}
