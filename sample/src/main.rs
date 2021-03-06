/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

extern crate app_units;
extern crate euclid;
extern crate gleam;
extern crate glutin;
extern crate webrender;
extern crate webrender_traits;

use app_units::Au;
use euclid::Point2D;
use gleam::gl;
use glutin::TouchPhase;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use webrender_traits::{BlobImageData, BlobImageDescriptor, BlobImageError, BlobImageRenderer};
use webrender_traits::{BlobImageResult, ClipRegion, ColorF, Epoch, GlyphInstance};
use webrender_traits::{DeviceIntPoint, DeviceUintSize, DeviceUintRect, LayoutPoint, LayoutRect, LayoutSize};
use webrender_traits::{ImageData, ImageDescriptor, ImageFormat, ImageKey, ImageRendering};
use webrender_traits::{PipelineId, RasterizedBlobImage};

#[derive(Debug)]
enum Gesture {
    None,
    Pan,
    Zoom,
}

#[derive(Debug)]
struct Touch {
    id: u64,
    start_x: f32,
    start_y: f32,
    current_x: f32,
    current_y: f32,
}

fn dist(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let dx = x0 - x1;
    let dy = y0 - y1;
    ((dx * dx) + (dy * dy)).sqrt()
}

impl Touch {
    fn distance_from_start(&self) -> f32 {
        dist(self.start_x, self.start_y, self.current_x, self.current_y)
    }

    fn initial_distance_from_other(&self, other: &Touch) -> f32 {
        dist(self.start_x, self.start_y, other.start_x, other.start_y)
    }

    fn current_distance_from_other(&self, other: &Touch) -> f32 {
        dist(self.current_x, self.current_y, other.current_x, other.current_y)
    }
}

struct TouchState {
    active_touches: HashMap<u64, Touch>,
    current_gesture: Gesture,
    start_zoom: f32,
    current_zoom: f32,
    start_pan: DeviceIntPoint,
    current_pan: DeviceIntPoint,
}

enum TouchResult {
    None,
    Pan(DeviceIntPoint),
    Zoom(f32),
}

impl TouchState {
    fn new() -> TouchState {
        TouchState {
            active_touches: HashMap::new(),
            current_gesture: Gesture::None,
            start_zoom: 1.0,
            current_zoom: 1.0,
            start_pan: DeviceIntPoint::zero(),
            current_pan: DeviceIntPoint::zero(),
        }
    }

    fn handle_event(&mut self, touch: glutin::Touch) -> TouchResult {
        match touch.phase {
            TouchPhase::Started => {
                debug_assert!(!self.active_touches.contains_key(&touch.id));
                self.active_touches.insert(touch.id, Touch {
                    id: touch.id,
                    start_x: touch.location.0 as f32,
                    start_y: touch.location.1 as f32,
                    current_x: touch.location.0 as f32,
                    current_y: touch.location.1 as f32,
                });
                self.current_gesture = Gesture::None;
            }
            TouchPhase::Moved => {
                match self.active_touches.get_mut(&touch.id) {
                    Some(active_touch) => {
                        active_touch.current_x = touch.location.0 as f32;
                        active_touch.current_y = touch.location.1 as f32;
                    }
                    None => panic!("move touch event with unknown touch id!")
                }

                match self.current_gesture {
                    Gesture::None => {
                        let mut over_threshold_count = 0;
                        let active_touch_count = self.active_touches.len();

                        for (_, touch) in &self.active_touches {
                            if touch.distance_from_start() > 8.0 {
                                over_threshold_count += 1;
                            }
                        }

                        if active_touch_count == over_threshold_count {
                            if active_touch_count == 1 {
                                self.start_pan = self.current_pan;
                                self.current_gesture = Gesture::Pan;
                            } else if active_touch_count == 2 {
                                self.start_zoom = self.current_zoom;
                                self.current_gesture = Gesture::Zoom;
                            }
                        }
                    }
                    Gesture::Pan => {
                        let keys: Vec<u64> = self.active_touches.keys().cloned().collect();
                        debug_assert!(keys.len() == 1);
                        let active_touch = &self.active_touches[&keys[0]];
                        let x = active_touch.current_x - active_touch.start_x;
                        let y = active_touch.current_y - active_touch.start_y;
                        self.current_pan.x = self.start_pan.x + x.round() as i32;
                        self.current_pan.y = self.start_pan.y + y.round() as i32;
                        return TouchResult::Pan(self.current_pan);
                    }
                    Gesture::Zoom => {
                        let keys: Vec<u64> = self.active_touches.keys().cloned().collect();
                        debug_assert!(keys.len() == 2);
                        let touch0 = &self.active_touches[&keys[0]];
                        let touch1 = &self.active_touches[&keys[1]];
                        let initial_distance = touch0.initial_distance_from_other(touch1);
                        let current_distance = touch0.current_distance_from_other(touch1);
                        self.current_zoom = self.start_zoom * current_distance / initial_distance;
                        return TouchResult::Zoom(self.current_zoom);
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.active_touches.remove(&touch.id).unwrap();
                self.current_gesture = Gesture::None;
            }
        }

        TouchResult::None
    }
}

fn load_file(name: &str) -> Vec<u8> {
    let mut file = File::open(name).unwrap();
    let mut buffer = vec![];
    file.read_to_end(&mut buffer).unwrap();
    buffer
}

struct Notifier {
    window_proxy: glutin::WindowProxy,
}

impl Notifier {
    fn new(window_proxy: glutin::WindowProxy) -> Notifier {
        Notifier {
            window_proxy: window_proxy,
        }
    }
}

impl webrender_traits::RenderNotifier for Notifier {
    fn new_frame_ready(&mut self) {
        #[cfg(not(target_os = "android"))]
        self.window_proxy.wakeup_event_loop();
    }

    fn new_scroll_frame_ready(&mut self, _composite_needed: bool) {
        #[cfg(not(target_os = "android"))]
        self.window_proxy.wakeup_event_loop();
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let res_path = if args.len() > 1 {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    let window = glutin::WindowBuilder::new()
                .with_title("WebRender Sample")
                .with_multitouch()
                .with_gl(glutin::GlRequest::GlThenGles {
                    opengl_version: (3, 2),
                    opengles_version: (3, 0)
                })
                .build()
                .unwrap();

    unsafe {
        window.make_current().ok();
    }

    let gl = match gl::GlType::default() {
        gl::GlType::Gl => unsafe { gl::GlFns::load_with(|symbol| window.get_proc_address(symbol) as *const _) },
        gl::GlType::Gles => unsafe { gl::GlesFns::load_with(|symbol| window.get_proc_address(symbol) as *const _) },
    };

    println!("OpenGL version {}", gl.get_string(gl::VERSION));
    println!("Shader resource path: {:?}", res_path);

    let (width, height) = window.get_inner_size().unwrap();

    let opts = webrender::RendererOptions {
        resource_override_path: res_path,
        debug: true,
        precache_shaders: true,
        blob_image_renderer: Some(Box::new(FakeBlobImageRenderer::new())),
        device_pixel_ratio: window.hidpi_factor(),
        .. Default::default()
    };

    let size = DeviceUintSize::new(width, height);
    let (mut renderer, sender) = webrender::renderer::Renderer::new(gl, opts, size).unwrap();
    let api = sender.create_api();

    let notifier = Box::new(Notifier::new(window.create_window_proxy()));
    renderer.set_render_notifier(notifier);

    let epoch = Epoch(0);
    let root_background_color = ColorF::new(0.3, 0.0, 0.0, 1.0);

    let vector_img = api.generate_image_key();
    api.add_image(
        vector_img,
        ImageDescriptor::new(100, 100, ImageFormat::RGBA8, true),
        ImageData::new_blob_image(Vec::new()),
        None,
    );

    let pipeline_id = PipelineId(0, 0);
    let mut builder = webrender_traits::DisplayListBuilder::new(pipeline_id);

    let bounds = LayoutRect::new(LayoutPoint::zero(), LayoutSize::new(width as f32, height as f32));
    builder.push_stacking_context(webrender_traits::ScrollPolicy::Scrollable,
                                  bounds,
                                  0,
                                  None,
                                  None,
                                  webrender_traits::MixBlendMode::Normal,
                                  Vec::new());
    builder.push_image(
        LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutSize::new(100.0, 100.0)),
        ClipRegion::simple(&bounds),
        LayoutSize::new(100.0, 100.0),
        LayoutSize::new(0.0, 0.0),
        ImageRendering::Auto,
        vector_img,
    );

    let sub_clip = {
        let mask_image = api.generate_image_key();
        api.add_image(
            mask_image,
            ImageDescriptor::new(2, 2, ImageFormat::A8, true),
            ImageData::new(vec![0, 80, 180, 255]),
            None,
        );
        let mask = webrender_traits::ImageMask {
            image: mask_image,
            rect: LayoutRect::new(LayoutPoint::new(75.0, 75.0), LayoutSize::new(100.0, 100.0)),
            repeat: false,
        };
        let complex = webrender_traits::ComplexClipRegion::new(
            LayoutRect::new(LayoutPoint::new(50.0, 50.0), LayoutSize::new(100.0, 100.0)),
            webrender_traits::BorderRadius::uniform(20.0));

        builder.new_clip_region(&bounds, vec![complex], Some(mask))
    };

    builder.push_rect(LayoutRect::new(LayoutPoint::new(100.0, 100.0), LayoutSize::new(100.0, 100.0)),
                      sub_clip,
                      ColorF::new(0.0, 1.0, 0.0, 1.0));
    builder.push_rect(LayoutRect::new(LayoutPoint::new(250.0, 100.0), LayoutSize::new(100.0, 100.0)),
                      sub_clip,
                      ColorF::new(0.0, 1.0, 0.0, 1.0));
    let border_side = webrender_traits::BorderSide {
        color: ColorF::new(0.0, 0.0, 1.0, 1.0),
        style: webrender_traits::BorderStyle::Groove,
    };
    let border_widths = webrender_traits::BorderWidths {
        top: 10.0,
        left: 10.0,
        bottom: 10.0,
        right: 10.0,
    };
    let border_details = webrender_traits::BorderDetails::Normal(webrender_traits::NormalBorder {
        top: border_side,
        right: border_side,
        bottom: border_side,
        left: border_side,
        radius: webrender_traits::BorderRadius::uniform(20.0),
    });
    builder.push_border(LayoutRect::new(LayoutPoint::new(100.0, 100.0), LayoutSize::new(100.0, 100.0)),
                        sub_clip,
                        border_widths,
                        border_details);


    if false { // draw text?
        let font_key = api.generate_font_key();
        let font_bytes = load_file("res/FreeSans.ttf");
        api.add_raw_font(font_key, font_bytes);

        let text_bounds = LayoutRect::new(LayoutPoint::new(100.0, 200.0), LayoutSize::new(700.0, 300.0));

        let glyphs = vec![
            GlyphInstance {
                index: 48,
                point: Point2D::new(100.0, 100.0),
            },
            GlyphInstance {
                index: 68,
                point: Point2D::new(150.0, 100.0),
            },
            GlyphInstance {
                index: 80,
                point: Point2D::new(200.0, 100.0),
            },
            GlyphInstance {
                index: 82,
                point: Point2D::new(250.0, 100.0),
            },
            GlyphInstance {
                index: 81,
                point: Point2D::new(300.0, 100.0),
            },
            GlyphInstance {
                index: 3,
                point: Point2D::new(350.0, 100.0),
            },
            GlyphInstance {
                index: 86,
                point: Point2D::new(400.0, 100.0),
            },
            GlyphInstance {
                index: 79,
                point: Point2D::new(450.0, 100.0),
            },
            GlyphInstance {
                index: 72,
                point: Point2D::new(500.0, 100.0),
            },
            GlyphInstance {
                index: 83,
                point: Point2D::new(550.0, 100.0),
            },
            GlyphInstance {
                index: 87,
                point: Point2D::new(600.0, 100.0),
            },
            GlyphInstance {
                index: 17,
                point: Point2D::new(650.0, 100.0),
            },
        ];

        builder.push_text(text_bounds,
                          webrender_traits::ClipRegion::simple(&bounds),
                          glyphs,
                          font_key,
                          ColorF::new(1.0, 1.0, 0.0, 1.0),
                          Au::from_px(32),
                          Au::from_px(0),
                          None);
    }

    builder.pop_stacking_context();

    api.set_root_display_list(
        Some(root_background_color),
        epoch,
        LayoutSize::new(width as f32, height as f32),
        builder.finalize(),
        true);
    api.set_root_pipeline(pipeline_id);
    api.generate_frame(None);

    let mut touch_state = TouchState::new();

    'outer: for event in window.wait_events() {
        let mut events = Vec::new();
        events.push(event);

        for event in window.poll_events() {
            events.push(event);
        }

        for event in events {
            match event {
                glutin::Event::Closed |
                glutin::Event::KeyboardInput(_, _, Some(glutin::VirtualKeyCode::Escape)) |
                glutin::Event::KeyboardInput(_, _, Some(glutin::VirtualKeyCode::Q)) => break 'outer,
                glutin::Event::KeyboardInput(glutin::ElementState::Pressed,
                                             _, Some(glutin::VirtualKeyCode::P)) => {
                    let enable_profiler = !renderer.get_profiler_enabled();
                    renderer.set_profiler_enabled(enable_profiler);
                    api.generate_frame(None);
                }
                glutin::Event::Touch(touch) => {
                    match touch_state.handle_event(touch) {
                        TouchResult::Pan(pan) => {
                            api.set_pan(pan);
                            api.generate_frame(None);
                        }
                        TouchResult::Zoom(zoom) => {
                            api.set_pinch_zoom(webrender_traits::ZoomFactor::new(zoom));
                            api.generate_frame(None);
                        }
                        TouchResult::None => {}
                    }
                }
                _ => ()
            }
        }

        renderer.update();
        renderer.render(DeviceUintSize::new(width, height));
        window.swap_buffers().ok();
    }
}

struct FakeBlobImageRenderer {
    images: HashMap<ImageKey, BlobImageResult>,
}

impl FakeBlobImageRenderer {
    fn new() -> Self {
        FakeBlobImageRenderer { images: HashMap::new() }
    }
}

impl BlobImageRenderer for FakeBlobImageRenderer {
    fn request_blob_image(&mut self,
                          key: ImageKey,
                          _: Arc<BlobImageData>,
                          descriptor: &BlobImageDescriptor,
                          _dirty_rect: Option<DeviceUintRect>) {
        let mut texels = Vec::with_capacity((descriptor.width * descriptor.height * 4) as usize);
        for y in 0..descriptor.height {
            for x in 0..descriptor.width {
                // render a simple checkerboard pattern
                let a = if (x % 20 >= 10) != (y % 20 >= 10) { 255 } else { 0 };
                match descriptor.format {
                    ImageFormat::RGBA8 => {
                        texels.push(a);
                        texels.push(a);
                        texels.push(a);
                        texels.push(255);
                    }
                    ImageFormat::A8 => {
                        texels.push(a);
                    }
                    _ => {
                        self.images.insert(key,
                            Err(BlobImageError::Other(format!(
                                "Usupported image format {:?}",
                                descriptor.format
                            )))
                        );
                        return;
                    }
                }
            }
        }

        self.images.insert(key, Ok(RasterizedBlobImage {
            data: texels,
            width: descriptor.width,
            height: descriptor.height,
        }));
    }

    fn resolve_blob_image(&mut self, key: ImageKey) -> BlobImageResult {
        self.images.remove(&key).unwrap_or(Err(BlobImageError::InvalidKey))
    }
}
