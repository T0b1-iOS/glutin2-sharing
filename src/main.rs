use std::{ffi::CString, num::NonZeroU32};

use glow::HasContext;
use glutin::{
    config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder},
    context::{ContextAttributesBuilder, NotCurrentContext, PossiblyCurrentContext},
    display::{Display, DisplayApiPreference, DisplayPicker},
    prelude::{GlDisplay, NotCurrentGlContextSurfaceAccessor, PossiblyCurrentGlContext},
    surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface},
};
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

struct ContextWrapper {
    window_surface: Surface<WindowSurface>,
    headless_surface: Surface<WindowSurface>,
    window: Option<NotCurrentContext>,
    headless: Option<NotCurrentContext>,
}

impl ContextWrapper {
    fn ct_wnd(&mut self) -> PossiblyCurrentContext {
        self.window
            .take()
            .unwrap()
            .make_current(&self.window_surface)
            .unwrap()
    }

    fn ct_head(&mut self) -> PossiblyCurrentContext {
        self.headless
            .take()
            .unwrap()
            .make_current(&self.headless_surface)
            .unwrap()
    }

    fn put_wnd(&mut self, ctx: PossiblyCurrentContext) {
        self.window = Some(ctx.make_not_current().unwrap())
    }

    fn put_head(&mut self, ctx: PossiblyCurrentContext) {
        self.headless = Some(ctx.make_not_current().unwrap())
    }
}

fn select_display_config(
    raw_display: RawDisplayHandle,
    raw_wnd: RawWindowHandle,
) -> (Display, Config) {
    // first try glx, then egl
    let mut display = unsafe {
        Display::from_raw(
            raw_display,
            DisplayPicker::new()
                .with_api_preference(DisplayApiPreference::Glx)
                .with_glx_error_registrar(Box::new(
                    winit::platform::unix::register_xlib_error_hook,
                )),
        )
    };
    if display.is_err() {
        display = unsafe {
            Display::from_raw(
                raw_display,
                DisplayPicker::new()
                    .with_api_preference(DisplayApiPreference::Egl)
                    .with_glx_error_registrar(Box::new(
                        winit::platform::unix::register_xlib_error_hook,
                    )),
            )
        };
    }
    let display = display.expect("No display backend found");

    let config = unsafe {
        display
            .find_configs(
                ConfigTemplateBuilder::new()
                    .compatible_with_native_window(raw_wnd)
                    .with_surface_type(ConfigSurfaceTypes::WINDOW)
                    .build(),
            )
            .unwrap()
            .next()
            .unwrap()
    };

    return (display, config);
}

fn create_surface(
    width: u32,
    height: u32,
    display: &Display,
    config: &Config,
    raw_wnd: RawWindowHandle,
) -> Surface<WindowSurface> {
    let width = NonZeroU32::new(width).unwrap();
    let height = NonZeroU32::new(height).unwrap();
    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_wnd, width, height);
    unsafe { display.create_window_surface(&config, &attrs).unwrap() }
}

fn main() {
    let event_loop = EventLoop::new();
    let raw_display = event_loop.raw_display_handle();

    let window = WindowBuilder::new().build(&event_loop).unwrap();
    let raw_wnd = window.raw_window_handle();

    let (display, config) = select_display_config(raw_display, raw_wnd);

    let mut width = window.inner_size().width;
    let mut height = window.inner_size().height;

    let mut ctx = {
        let headless_context = unsafe {
            display
                .create_context(&config, &ContextAttributesBuilder::new().build())
                .unwrap()
        };

        let windowed_context = unsafe {
            display
                .create_context(
                    &config,
                    &ContextAttributesBuilder::new()
                        .with_sharing(&headless_context)
                        .build_windowed(raw_wnd),
                )
                .unwrap()
        };

        let window_surface = create_surface(width, height, &display, &config, raw_wnd);
        let headless_surface = create_surface(1, 1, &display, &config, raw_wnd);

        ContextWrapper {
            window_surface,
            headless_surface,
            window: Some(windowed_context),
            headless: Some(headless_context),
        }
    };

    let c = ctx.ct_wnd();

    let glw = unsafe {
        glow::Context::from_loader_function(|s| {
            c.get_proc_address(CString::new(s).unwrap().as_c_str())
                .cast()
        })
    };

    let render_buf = {
        let render_buf = unsafe { glw.create_renderbuffer().unwrap() };
        unsafe {
            glw.bind_renderbuffer(glow::RENDERBUFFER, Some(render_buf));
            glw.renderbuffer_storage(glow::RENDERBUFFER, glow::RGB8, width as _, height as _);
        }

        render_buf
    };

    let window_fb = unsafe { glw.create_framebuffer().unwrap() };
    unsafe {
        glw.bind_framebuffer(glow::FRAMEBUFFER, Some(window_fb));
        glw.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(render_buf),
        );
        glw.bind_framebuffer(glow::FRAMEBUFFER, None);
        glw.viewport(0, 0, width as _, height as _);
    }

    ctx.put_wnd(c);

    let c = ctx.ct_head();
    let glh = unsafe {
        glow::Context::from_loader_function(|s| {
            c.get_proc_address(CString::new(s).unwrap().as_c_str())
                .cast()
        })
    };

    let headless_fb = unsafe { glh.create_framebuffer().unwrap() };
    unsafe {
        glh.bind_framebuffer(glow::FRAMEBUFFER, Some(headless_fb));
        glh.bind_renderbuffer(glow::RENDERBUFFER, Some(render_buf));
        glh.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(render_buf),
        );
        glh.viewport(0, 0, width as _, height as _);
    }

    ctx.put_head(c);

    event_loop.run(move |event, _, cf| {
        println!("{:?}", event);
        *cf = ControlFlow::Wait;

        match event {
            Event::LoopDestroyed => {
                let c = ctx.ct_wnd();
                unsafe {
                    glw.delete_framebuffer(window_fb);
                    glw.delete_renderbuffer(render_buf);
                }
                ctx.put_wnd(c);
                let c = ctx.ct_head();
                unsafe {
                    glh.delete_framebuffer(headless_fb);
                }
                ctx.put_head(c);
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::Resized(size) => {
                    width = size.width;
                    height = size.height;

                    let c = ctx.ct_wnd();
                    ctx.window_surface.resize(
                        &c,
                        NonZeroU32::new(width).unwrap(),
                        NonZeroU32::new(height).unwrap(),
                    );
                    ctx.window_surface.swap_buffers(&c).unwrap();
                    unsafe {
                        glw.renderbuffer_storage(
                            glow::RENDERBUFFER,
                            glow::RGB8,
                            width as _,
                            height as _,
                        );
                        glw.viewport(0, 0, width as _, height as _);
                    }
                    ctx.put_wnd(c);

                    let c = ctx.ct_head();
                    ctx.headless_surface.resize(
                        &c,
                        NonZeroU32::new(width).unwrap(),
                        NonZeroU32::new(height).unwrap(),
                    );
                    ctx.headless_surface.swap_buffers(&c).unwrap();

                    unsafe {
                        glh.viewport(0, 0, width as _, height as _);
                    }
                    ctx.put_head(c);
                    window.request_redraw();
                }
                WindowEvent::CloseRequested => *cf = ControlFlow::Exit,
                _ => {}
            },
            Event::RedrawRequested(_) => {
                let c = ctx.ct_head();
                unsafe {
                    glh.clear_color(1.0, 0.5, 0.7, 1.0);
                    glh.clear(glow::COLOR_BUFFER_BIT);
                }
                ctx.headless_surface.swap_buffers(&c).unwrap();
                ctx.put_head(c);

                let c = ctx.ct_wnd();
                unsafe {
                    glw.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(headless_fb));
                    glw.blit_framebuffer(
                        0,
                        0,
                        width as _,
                        height as _,
                        0,
                        0,
                        width as _,
                        height as _,
                        glow::COLOR_BUFFER_BIT,
                        glow::NEAREST,
                    );
                }

                ctx.window_surface.swap_buffers(&c).unwrap();
                ctx.put_wnd(c);
            }
            _ => {}
        }
    });
}
