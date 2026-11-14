use forge_ui::backend::{CoreBackend, OfflineBackend};
use std::rc::Rc;

fn main() {
    gpui_kit::platform::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            forge_ui::init(cx);
            match CoreBackend::start(forge_ui::backend::data_dir()) {
                Ok((backend, events)) => {
                    forge_ui::open_main_window(cx, Rc::new(backend), Some(events), None)
                }
                Err(error) => {
                    forge_ui::open_main_window(cx, Rc::new(OfflineBackend), None, Some(error))
                }
            }
            cx.activate(true);
        });
}
