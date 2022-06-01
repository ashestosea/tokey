/*
* Copyright © 2022 Damian Geerdes (chronotab) <damian.geerdes@tutanota.com>
* This work is free. You can redistribute it and/or modify it under the
* terms of the Do What The Fuck You Want To Public License, Version 2,
* as published by Sam Hocevar. See the COPYING file for more details.
*/

use dbus::arg;
use dbus::blocking::Connection;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_crossroads::{Crossroads, IfaceBuilder};
use evdev::InputEvent;
use evdev::InputEventKind;
use evdev::Key;
use nix::{
    fcntl::{FcntlArg, OFlag},
    sys::epoll,
};
use serde_derive::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::process::exit;
use std::str::FromStr;
use std::time::Duration;
use std::time::Instant;
use toml;

extern crate xdg;

enum State {
    IDLE,
    DECIDE,
    SHIFT,
}

enum KeyState {
    INVALID = -1,
    UP = 0,
    DOWN = 1,
    REPEAT = 2,
}

impl Into<KeyState> for i32 {
    fn into(self) -> KeyState {
        match self {
            -1 => KeyState::INVALID,
            0 => KeyState::UP,
            1 => KeyState::DOWN,
            2 => KeyState::REPEAT,
            _ => KeyState::INVALID,
        }
    }
}

#[derive(Deserialize)]
struct Config {
    device_name: toml::Value,
    mode_switch_timeout: toml::Value,
    fn_key: toml::Value,
    pause_key: toml::Value,
    keymap: toml::value::Table,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DBUS_IFACE_NAME: &str = "com.spacefn.spacefn";
const DBUS_PATH: &str = "/";
const DBUS_PROP_NAME: &str = "Paused";

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
    println!(
        r#"Usage: spacefn-rs [OPTION]... [FILE]...
Add Description of spacefn

  -c,            specify a custom configuration file
  -v, --help     display this help and exit
      --version  output version information and exit

Full documentation <https://www.github.com/chronotab/spacefn-rs>
    "#
    );
    exit(1);
}

fn get_config() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let mut conf_contents = String::new();

    match &args.len() {
        // no Arguments passed
        1 => {
            let xdg_dirs = xdg::BaseDirectories::with_prefix("spacefn-rs").unwrap();
            let conf_filename_opt = xdg_dirs.find_config_file("conf.toml");
            if conf_filename_opt.is_none() {
                let conf_path = xdg_dirs
                    .place_config_file("conf.toml")
                    .expect("Can't create config directory");
                let mut conf_file = std::fs::File::create(conf_path).unwrap();
                write!(&mut conf_file, default_conf!()).expect("Can't write config file");
            }

            conf_contents = String::from_str(default_conf!()).unwrap();
        }
        2 => {
            if &args[1] == "-v" {
                version();
            } else {
                help();
            }
        }
        // flag and argument passed
        3 => match args[1].as_str() {
            "-c" => {
                conf_contents = std::fs::read_to_string(&args[2])
                    .expect("Something went wrong reading the file");
            }
            _ => {
                help();
            }
        },
        _ => {
            help();
        }
    }

    toml::from_str::<Config>(conf_contents.as_str()).expect("Error parsing config file")
}

fn get_keymap(in_keymap: toml::value::Map<String, toml::Value>) -> HashMap<u16, u16> {
    let mut keymap: HashMap<u16, u16> = HashMap::new();
    for kvp in in_keymap.iter() {
        let k = Key::from_str(kvp.0).expect(format!("Invalid keymap key").as_str());
        let v_str = kvp
            .1
            .as_str()
            .expect(format!("Couldn't parse keymap value as string").as_str());
        let v = Key::from_str(v_str).expect(format!("Invalid keymap value").as_str());
        keymap.insert(k.code(), v.code());
    }
    return keymap;
}

fn get_device(mut device_name: String) -> std::io::Result<evdev::Device> {
    let device: evdev::Device;
    device_name.remove(0);
    device_name.remove(device_name.len() - 1);

    if device_name.starts_with("/dev/input/") {
        device = evdev::Device::open(device_name).unwrap();
    } else {
        device = evdev::enumerate()
            .find(|d| d.name().unwrap().contains(&device_name))
            .unwrap();
    }

    let raw_fd = device.as_raw_fd();
    nix::fcntl::fcntl(raw_fd, FcntlArg::F_SETFL(OFlag::O_RDONLY))?;

    // create epoll handle and attach raw_fd
    let epoll_fd = epoll::epoll_create1(epoll::EpollCreateFlags::EPOLL_CLOEXEC)?;
    let mut event = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
    epoll::epoll_ctl(
        epoll_fd.as_raw_fd(),
        epoll::EpollOp::EpollCtlAdd,
        raw_fd,
        Some(&mut event),
    )?;

    Ok(device)
}

fn register_dbus_iface() -> Result<(), Box<dyn std::error::Error>> {
    let c = Connection::new_session()?;
    c.request_name(DBUS_IFACE_NAME, false, true, false)?;
    
    let mut cr = Crossroads::new();
    
    let token = cr.register(DBUS_IFACE_NAME, |f: &mut IfaceBuilder<bool>| {
        f.property(DBUS_PROP_NAME)
            .get(|_, data| Ok(*data))
            .set(|_, data, value| {
                *data = value;
                Ok(Some(value))
            });
    });
    
    cr.insert(DBUS_PATH, &[token], false);
    
    let _ = &c.start_receive(MatchRule::new_method_call(), Box::new(move |msg, conn| {
        cr.handle_message(msg, conn).unwrap();
        true
    }));
    
    std::thread::spawn(move || {
        loop {
            match c.process(Duration::from_millis(1000)) {
                Ok(_) => {}
                Err(err) => {
                    println!("dbus loop error: {}", err);
                    break
                }
            }
        }
    });
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // setup
    let config = get_config();
    let mode_switch_timeout = config
        .mode_switch_timeout
        .as_integer()
        .expect("Invalid mode_switch_timeout") as u64;
    let fn_key = Key::from_str(config.fn_key.as_str().unwrap()).expect("Invalid fn_key");
    let pause_key = Key::from_str(config.pause_key.as_str().unwrap()).expect("Invalid pause_key");
    let keymap = get_keymap(config.keymap);
    let mut dev = get_device(config.device_name.to_string()).expect("Invalid input device");
    let mut virt_dev = evdev::uinput::VirtualDeviceBuilder::new()?
        .name("spacefn-kbd")
        .with_keys(dev.supported_keys().unwrap())?
        .build()
        .unwrap();
        
    register_dbus_iface()?;
    
    let mut state = State::IDLE;
    let timeout: Duration = Duration::from_millis(mode_switch_timeout);
    let mut start_time = Instant::now();
    let key_event = evdev::EventType::KEY;
    let mut event_buffer = vec![0; 10];
    
    let c2 = Connection::new_session()?;
    let _ = dev.grab();
    loop {
        match dev.fetch_events() {
            Ok(iterator) => {
                for ev in iterator {
                    if ev.code() == 0 || ev.event_type() != key_event {
                        continue;
                    }
                    match state {
                        State::IDLE => {
                            let p = c2.with_proxy(
                                DBUS_IFACE_NAME,
                                DBUS_PATH,
                                Duration::from_millis(1000));
                            let paused_refarg = p.get::<Box<dyn arg::RefArg>>(
                                DBUS_IFACE_NAME,
                                 DBUS_PROP_NAME
                                )?;
                            let paused: bool = *arg::cast::<bool>(&paused_refarg).unwrap();
                            if ev.kind() == InputEventKind::Key(pause_key)
                            && ev.value() == KeyState::DOWN as i32
                            {
                                p.set(DBUS_IFACE_NAME,
                                    DBUS_PROP_NAME,
                                    !paused)?;
                                break;
                            } else if ev.kind() == InputEventKind::Key(fn_key) && !paused {
                                start_time = Instant::now();
                                state = State::DECIDE;
                                break;
                            }
                            virt_dev.emit(&[ev]).unwrap();
                        }
                        State::DECIDE => {
                            let current_time = Instant::now();
                            if current_time.duration_since(start_time) >= timeout {
                                // Send all buffered key events as down
                                for i in &event_buffer {
                                    let e = InputEvent::new(
                                        key_event,
                                        *i,
                                        KeyState::DOWN as i32);
                                    virt_dev.emit(&[e]).unwrap();
                                }
                                event_buffer.clear();
                                state = State::SHIFT;
                                break;
                            } else {
                                if ev.value() == KeyState::DOWN as i32 {
                                    // add to event buffer
                                    event_buffer.push(ev.code());
                                } else if ev.value() == KeyState::UP as i32 {
                                    let mut code = ev.code();
                                    if ev.kind() == InputEventKind::Key(fn_key) {
                                        let fn_down = InputEvent::new(
                                            key_event,
                                            code,
                                            KeyState::DOWN as i32,
                                        );
                                        let fn_up = InputEvent::new(
                                            key_event,
                                            code,
                                            KeyState::UP as i32,
                                        );

                                        virt_dev.emit(&[fn_down]).unwrap();
                                        virt_dev.emit(&[fn_up]).unwrap();
                                        // Send all buffered key events as down
                                        for i in &event_buffer {
                                            let e = InputEvent::new(
                                                key_event,
                                                *i,
                                                KeyState::DOWN as i32,
                                            );
                                            virt_dev.emit(&[e]).unwrap();
                                        }
                                        event_buffer.clear();
                                        state = State::IDLE;
                                        break;
                                    } else if event_buffer.contains(&code) {
                                        // remove ev from buffer
                                        event_buffer.retain(|c| c != &code);
                                        if keymap.contains_key(&code) {
                                            code = keymap[&code];
                                        }
                                        
                                        let ev_down = InputEvent::new(
                                            key_event,
                                            code,
                                            KeyState::DOWN as i32,
                                        );
                                        let ev_up = InputEvent::new(
                                            key_event,
                                            code,
                                            KeyState::UP as i32,
                                        );

                                        virt_dev.emit(&[ev_down]).unwrap();
                                        virt_dev.emit(&[ev_up]).unwrap();
                                        state = State::SHIFT;
                                        break;
                                    } else {
                                        // key was pressed before fn_key
                                        virt_dev.emit(&[ev]).unwrap();
                                    }
                                }
                            }
                        }
                        State::SHIFT => {
                            if ev.kind() == InputEventKind::Key(fn_key) {
                                if ev.value() == KeyState::UP as i32 {
                                    // Send all buffered key events as up
                                    for i in &event_buffer {
                                            let e = InputEvent::new(
                                                key_event,
                                                *i,
                                                KeyState::UP as i32,
                                            );
                                            virt_dev.emit(&[e]).unwrap();
                                    }
                                    event_buffer.clear();
                                    state = State::IDLE;
                                }
                            }

                            if keymap.contains_key(&ev.code()) {
                                let mapped_ev =
                                    InputEvent::new(
                                        key_event,
                                        keymap[&ev.code()],
                                        ev.value());

                                match ev.value().into() {
                                    KeyState::UP => {
                                        // remove ev from buffer
                                        event_buffer.retain(|c| c != &mapped_ev.code());
                                    }
                                    KeyState::DOWN => {
                                        event_buffer.push(mapped_ev.code());
                                    }
                                    _ => {}
                                }

                                virt_dev.emit(&[mapped_ev]).unwrap();
                            } else {
                                virt_dev.emit(&[ev]).unwrap();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                break;
            }
        }
    }

    dev.ungrab()?;
    Ok(())
}