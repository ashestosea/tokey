/*
* Copyright © 2022 Damian Geerdes (chronotab) <damian.geerdes@tutanota.com>
* This work is free. You can redistribute it and/or modify it under the
* terms of the Do What The Fuck You Want To Public License, Version 2,
* as published by Sam Hocevar. See the COPYING file for more details.
*/

use evdev::Key as Key;
use serde_derive::Deserialize;
use std::format;
use std::env;
use std::fs;
use std::fs::File;
use std::collections::HashMap;
use std::io::Write;
use std::process::exit;
use std::str::FromStr;
use toml;

extern crate xdg;

#[derive(Deserialize)]
struct Config {
    device: toml::Value,
    keymap: toml::value::Table,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! default_conf {
    () => {
        "device = \"\"\n[keymap]\n"
    };
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut conf_contents = String::new();
    
    match &args.len() {
        // No Arguments passed
        1 => {
            let xdg_dirs = xdg::BaseDirectories::with_prefix("spacefn-rs").unwrap();
            let conf_filename_opt = xdg_dirs.find_config_file("conf.toml");
            if conf_filename_opt.is_none() {
                let conf_path = xdg_dirs.place_config_file("conf.toml")
                    .expect("Can't create config directory");
                let mut conf_file = File::create(conf_path).unwrap();
                write!(&mut conf_file, default_conf!())
                    .expect("Can't write config file");
            }
            
            conf_contents = String::from_str(default_conf!()).unwrap();
        }
        2 => {
            if &args[1] == "-v" { version(); }
            else { help(); }
        }
        // Flag and argument passed
        3 => {
            match args[1].as_str() {
                "-c" => {
                    conf_contents = fs::read_to_string(&args[2])
                        .expect("Something went wrong reading the file");
                }
                _ => { help(); }
            }
        }
        _ => { help(); }
    }
    
    let config: Config = toml::from_str(conf_contents.as_str())
        .expect("Error parsing config file");
    
    let mut keymap: HashMap<Key, Key> = HashMap::new();
    let kvp_index: u16 = 0;
    for kvp in config.keymap.iter() {
        let k = Key::from_str(kvp.0)
            .expect(format!("Invalid keymap key (keymap index {})", kvp_index).as_str());
        let v_str = kvp.1.as_str()
            .expect(format!("Couldn't parse keymap value as string (keymap index {})", kvp_index).as_str());
        let v = Key::from_str(v_str)
            .expect(format!("Invalid keymap value (keymap index {})", kvp_index).as_str());
        keymap.insert(k, v);
    }
    
    println!("---Keymap---");
    for kvp in keymap.iter() {
        println!("key = {} :: val = {}", kvp.0.code(), kvp.1.code());
    }
    
    println!("\nListening for events on device {}", config.device);
}

fn version() {
    println!("Version: {}", VERSION);
    exit(0);
}

fn help() {
    println!(r#"Usage: spacefn-rs [OPTION]... [FILE]...
Add Description of spacefn

  -c,            specify a custom configuration file
  -v, --help     display this help and exit
      --version  output version information and exit

Full documentation <https://www.github.com/chronotab/spacefn-rs>
    "#);
    exit(1);
}