#![forbid(unsafe_code)]
use hengscore_poc::{
    registration::{Fault, Store},
    submission::MAX_RAW,
};
use std::{env, error::Error, fs::File, io::Read, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let fault_at = args.iter().position(|s| s == "--fail-at");
    let (args, fault) = match fault_at {
        Some(index) if index + 2 == args.len() => {
            (&args[..index], Fault(Some(args[index + 1].clone())))
        }
        None => (&args[..], Fault::default()),
        _ => return Err("invalid fault arguments".into()),
    };
    fault.validate()?;
    match args {
        [command, root, token, header, input] if command == "register" => {
            let mut raw = Vec::new();
            File::open(input)?.take((MAX_RAW + 1) as u64).read_to_end(&mut raw)?;
            let mut store = Store::open(Path::new(root))?;
            let id = store.register(token, header, &raw, &fault)?;
            println!("REGISTERED id={id}");
        }
        [command, root] if command == "recover" => {
            let mut store = Store::open(Path::new(root))?;
            println!("RECOVERED count={}", store.recover(&fault)?);
        }
        _ => return Err("usage: register_probe register ROOT TOKEN CONTENT_TYPE FILE [--fail-at KIND:POINT] | recover ROOT".into()),
    }
    Ok(())
}
