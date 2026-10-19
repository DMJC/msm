use gtk::prelude::*;

pub fn build(app: &gtk::Application) {
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Matrix Messenger")
        .default_width(320)
        .default_height(640)
        .build();
    win.add(&gtk::Label::new(Some("Matrix Messenger")));
    win.show_all();
}
