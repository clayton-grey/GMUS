use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyAccessory, NSEventMask};
use cocoa::base::{nil, YES};
use cocoa::foundation::{NSAutoreleasePool, NSDate, NSDefaultRunLoopMode};

pub(super) struct AppKitPump;

impl AppKitPump {
    pub(super) fn new() -> Self {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let app = NSApp();
            let _ = app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
            app.finishLaunching();
            pool.drain();
        }
        Self
    }

    pub(super) fn pump_pending_events(&self) {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let app = NSApp();
            let until = NSDate::distantPast(nil);
            loop {
                let event = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                    NSEventMask::NSAnyEventMask.bits(),
                    until,
                    NSDefaultRunLoopMode,
                    YES,
                );
                if event == nil {
                    break;
                }
                app.sendEvent_(event);
            }
            pool.drain();
        }
    }
}
