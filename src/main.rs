extern crate debug_print;
extern crate getopts;

use debug_print::debug_println as dprintln;
use getopts::Options;
use std::env;

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [OPTIONS] URIs...", program);
    print!("{}", opts.usage(&brief));
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();

    opts.optflag("h", "help", "print the help menu");
    opts.optopt("u", "user", "user login name, required", "USER");
    opts.optopt("p", "password", "user password, required", "PASS");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            panic!("{}", f.to_string())
        }
    };

    let input_uris = matches.free.clone();

    if matches.opt_present("h")
        || !matches.opt_present("u")
        || !matches.opt_present("p")
        || input_uris.len() == 0
    {
        print_usage(&program, opts);
        return;
    }

    let user = matches.opt_str("u").unwrap();
    let pass = matches.opt_str("p").unwrap();

    dprintln!("user: {}, pass: {}", user, pass);

    for uri in input_uris {
        dprintln!("parsing uri: {}", uri);
    }
}
