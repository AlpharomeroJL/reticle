//! Native launcher for the Reticle application.

use reticle_app::App;

fn main() {
    // Wave 4: create the egui/winit event loop and run the app.
    let app = App::new();
    println!(
        "reticle-app (Wave 4 stub): {} frame(s) rendered",
        app.renderer().frames_rendered()
    );
}
