use std::path::PathBuf;

/// Exercise an installed distribution when requested; otherwise use Cargo's binary.
pub fn binary(name: &str, built: &str) -> PathBuf {
    match std::env::var_os("DAML_TEST_BIN_DIR") {
        Some(directory) => {
            let mut path = PathBuf::from(directory).join(name);
            if cfg!(windows) {
                path.set_extension("exe");
            }
            path
        }
        None => PathBuf::from(built),
    }
}
