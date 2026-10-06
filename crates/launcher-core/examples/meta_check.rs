//! Temporary diagnostic: parse real version documents and report the argument
//! shapes found. Usage: `cargo run -p launcher-core --example meta_check -- a.json b.json`

use launcher_core::{ArgumentValue, VersionMeta};

fn count_unknown(values: Option<&Vec<ArgumentValue>>) -> usize {
    values
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|value| matches!(value, ArgumentValue::Unknown(_)))
        .count()
}

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("READ-FAIL {path}: {error}");
                continue;
            }
        };
        match serde_json::from_slice::<VersionMeta>(&bytes) {
            Ok(meta) => {
                let arguments = meta.arguments.as_ref();
                println!(
                    "OK   {path} id={} game={} jvm={} default-user-jvm={} unknown={}",
                    meta.id,
                    arguments
                        .and_then(|value| value.game.as_ref())
                        .map(Vec::len)
                        .unwrap_or(0),
                    arguments
                        .and_then(|value| value.jvm.as_ref())
                        .map(Vec::len)
                        .unwrap_or(0),
                    arguments
                        .and_then(|value| value.default_user_jvm.as_ref())
                        .map(Vec::len)
                        .unwrap_or(0),
                    count_unknown(arguments.and_then(|value| value.game.as_ref()))
                        + count_unknown(arguments.and_then(|value| value.jvm.as_ref()))
                        + count_unknown(
                            arguments.and_then(|value| value.default_user_jvm.as_ref())
                        ),
                );
            }
            Err(error) => println!("FAIL {path}: {error}"),
        }
    }
}
