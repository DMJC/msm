mod contacts;
mod storage;
mod ui;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let app = gtk::Application::builder()
        .application_id("com.matrixmessenger.Msm")
        .build();
    app.connect_activate(ui::main_window::build);
    app.run()
}
