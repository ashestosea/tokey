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
use std::collections::HashMap;
use std::str::FromStr;
use toml;

#[derive(Deserialize)]
struct Config {
    device: toml::Value,
    keymap: toml::value::Table,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let config_filename = &args[1];
    
    let config_file_contents = fs::read_to_string(config_filename)
    .expect("Something went wrong reading the file");
    
    let config: Config = toml::from_str(config_file_contents.as_str())
    .expect("Error parsing config file");
    
    let mut keymap: HashMap<Key, Key> = HashMap::new();
    let kvp_index: u16 = 0;
    for kvp in config.keymap.iter() {
        // println!("key = {} :: val = {}", kvp.0, kvp.1.to_string());
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