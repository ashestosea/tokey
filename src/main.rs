/*
* Copyright © 2022 Damian Geerdes (chronotab) <damian.geerdes@tutanota.com>
* This work is free. You can redistribute it and/or modify it under the
* terms of the Do What The Fuck You Want To Public License, Version 2,
* as published by Sam Hocevar. See the COPYING file for more details.
*/

use evdev::InputEvent;
use evdev::Key as Key;
use nix::{fcntl::{FcntlArg, OFlag}, sys::epoll};
use serde_derive::Deserialize;
use std::collections::HashMap;
use std::env;
use std::format;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::process::exit;
use std::str::FromStr;
use toml;

extern crate xdg;

#[derive(Deserialize)]
struct Config {
    device_name: toml::Value,
    toggle_state_key: toml::Value,
    keymap: toml::value::Table,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! default_conf {
    () => {
r#"# If device_name starts with /dev/input/ it's treated as a path
# Else we grab the highest numbered device with a name that contains device_name
device = ""

toggle_state_key = "KEY_RIGHTMETA"

[keymap]
"#
    };
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

fn get_config() -> Config {
    let args: Vec<String> = env::args().collect();
    let mut conf_contents = String::new();
    
    match &args.len() {
        // no Arguments passed
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
        // flag and argument passed
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
    
    toml::from_str::<Config>(conf_contents.as_str())
        .expect("Error parsing config file")
}

fn get_keymap(in_keymap: toml::value::Map<String, toml::Value>) -> HashMap<u16, u16> {
    let mut keymap: HashMap<u16, u16> = HashMap::new();
    let kvp_index: u16 = 0;
    for kvp in in_keymap.iter() {
        let k = Key::from_str(kvp.0)
            .expect(format!("Invalid keymap key (keymap index {})", kvp_index).as_str());
        let v_str = kvp.1.as_str()
            .expect(format!("Couldn't parse keymap value as string (keymap index {})", kvp_index).as_str());
        let v = Key::from_str(v_str)
            .expect(format!("Invalid keymap value (keymap index {})", kvp_index).as_str());
        keymap.insert(k.code(), v.code());
        // println!("key {} :: val {}", k.code(), v.code());
    }
	return keymap;
}

fn get_device(mut device_name: String) -> std::io::Result<evdev::Device> {
    let device: evdev::Device;
    device_name.remove(0);
    device_name.remove(device_name.len() - 1);
    
    if device_name.starts_with("/dev/input/") {
        device = evdev::Device::open(device_name).unwrap();
    }
    else {
        device = evdev::enumerate().find(|d| d.name().unwrap().contains(&device_name)).unwrap();
    }
    
    let raw_fd = device.as_raw_fd();
    nix::fcntl::fcntl(raw_fd, FcntlArg::F_SETFL(OFlag::O_RDONLY))?;

    // create epoll handle and attach raw_fd
    let epoll_fd = epoll::epoll_create1(
        epoll::EpollCreateFlags::EPOLL_CLOEXEC,
    )?;
    let mut event = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
    epoll::epoll_ctl(
        epoll_fd.as_raw_fd(),
        epoll::EpollOp::EpollCtlAdd,
        raw_fd,
        Some(&mut event),
    )?;
	
	Ok(device)
}

fn main() -> std::io::Result<()> {
	// setup
	let config = get_config();
	let keymap = get_keymap(config.keymap);
	let toggle_state_key: Key = Key::from_str(config.toggle_state_key.as_str().unwrap()).unwrap();
	let mut dev = get_device(config.device_name.to_string())?;
    let mut virt_dev = evdev::uinput::VirtualDeviceBuilder::new()?
        .name("spacefn-kbd")
        .with_keys(dev.supported_keys().unwrap())?
        .build()
        .unwrap();
    
    // main loop
    let _ = &dev.grab();
    let mut run = true;
    while run {
        match dev.fetch_events() {
            Ok(iterator) => {
                for ev in iterator {
                    // println!("{:?}", ev);
                    if keymap.contains_key(&ev.code()) {
                        let key_event = InputEvent::new(
							evdev::EventType::KEY,
							keymap[&ev.code()], ev.value());
                        virt_dev.emit(&[key_event]).unwrap();
                    }
                    else if ev.code() == toggle_state_key.code() {
                        run = false;
                    }
                    else {
                        virt_dev.emit(&[ev]).unwrap();
                    }
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                break;
            }
        }
    }
    
    let _ = &dev.ungrab();
    Ok(())
}
