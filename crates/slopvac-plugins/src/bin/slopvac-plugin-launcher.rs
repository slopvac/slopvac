//! The plugin launcher as a program of its own, which this package's tests
//! start plugins through. The `slopvac` binary runs the same code.

fn main() {
    slopvac_plugins::launch::intercept();
    eprintln!(
        "usage: slopvac-plugin-launcher {} ... (started by the plugin host only)",
        slopvac_plugins::launch::FLAG
    );
    std::process::exit(2);
}
