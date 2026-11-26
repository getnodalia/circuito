// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `circuito` command. All the work is in the library so it can be tested in process.

use std::io::Write;

fn main() {
    let mut args = Vec::new();
    for a in std::env::args_os().skip(1) {
        match a.into_string() {
            Ok(s) => args.push(s),
            Err(_) => {
                let _ = writeln!(
                    std::io::stderr(),
                    "circuito: an argument is not valid UTF-8"
                );
                std::process::exit(2);
            }
        }
    }
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = circuito_cli::run(&args, &mut stdout.lock(), &mut stderr.lock());
    std::process::exit(code);
}
