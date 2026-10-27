mod icons;
mod markdown;
mod model;
mod shell;
mod terminal;

fn main() {
    if std::env::args().any(|arg| arg == "--web-ui") {
        shell::run_web();
    } else {
        shell::run();
    }
}
