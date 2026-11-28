#![allow(unexpected_cfgs)]

#[derive(Clone, Copy, Debug)]
pub(crate) struct Pinch {
    pub magnification: f32,
    pub x: f32,
    pub y_from_bottom: f32,
}

#[cfg(all(target_os = "macos", not(test)))]
pub(crate) fn listen() -> Option<async_channel::Receiver<Pinch>> {
    use block::ConcreteBlock;
    use objc::{Encode, Encoding, class, msg_send, runtime::Object, sel, sel_impl};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Location {
        x: f64,
        y: f64,
    }

    unsafe impl Encode for Location {
        fn encode() -> Encoding {
            unsafe { Encoding::from_str("{CGPoint=dd}") }
        }
    }

    const MAGNIFY: u64 = 1 << 30;
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    let (sender, receiver) = async_channel::unbounded();
    let handler = ConcreteBlock::new(move |event: *mut Object| -> *mut Object {
        if !event.is_null() {
            let (magnification, location) = unsafe {
                let magnification: f64 = msg_send![event, magnification];
                let location: Location = msg_send![event, locationInWindow];
                (magnification, location)
            };
            let _ = sender.try_send(Pinch {
                magnification: magnification as f32,
                x: location.x as f32,
                y_from_bottom: location.y as f32,
            });
        }
        event
    })
    .copy();
    let monitor: *mut Object = unsafe {
        msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: MAGNIFY handler: &*handler]
    };
    std::mem::forget(handler);
    (!monitor.is_null()).then_some(receiver)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn listen() -> Option<async_channel::Receiver<Pinch>> {
    None
}
