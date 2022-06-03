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
use evdev::uinput::VirtualDevice;
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

fn send_key_down(virt_dev: &mut VirtualDevice, code: u16) {
    send_key(virt_dev, code, KeyState::DOWN);
}

fn send_key_up(virt_dev: &mut VirtualDevice, code: u16) {
    send_key(virt_dev, code, KeyState::UP);
}

fn send_key(virt_dev: &mut VirtualDevice, code: u16, value: KeyState) {
    let event = InputEvent::new(evdev::EventType::KEY, code, value as i32);
    virt_dev.emit(&[event]).unwrap();
}

fn send_key_i32(virt_dev: &mut VirtualDevice, code: u16, value: i32) {
    let event = InputEvent::new(evdev::EventType::KEY, code, value);
    virt_dev.emit(&[event]).unwrap();
}

struct StateMachine {
    state: State,
    conn: Connection,
    virt_dev: VirtualDevice,
    fn_key: Key,
    pause_key: Key,
    keymap: HashMap<u16, u16>,
    timeout: Duration,
    start_time: Instant,
    event_buffer: Vec<u16>
}

impl StateMachine {
    fn new(
        conn: Connection,
        virt_dev: VirtualDevice,
        config: Config) -> Self {
        let fn_key = Key::from_str(config.fn_key.as_str().unwrap()).expect("Invalid fn_key");
        let pause_key = Key::from_str(config.pause_key.as_str().unwrap()).expect("Invalid pause_key");
        let keymap = get_keymap(config.keymap);
        let mode_switch_timeout = config
            .mode_switch_timeout
            .as_integer()
            .expect("Invalid mode_switch_timeout") as u64;
        let timeout: Duration = Duration::from_millis(mode_switch_timeout);
        let start_time = Instant::now();
        let event_buffer = vec![0; 10];
        StateMachine {
            state: State::IDLE,
            conn,
            virt_dev,
            fn_key,
            pause_key,
            keymap,
            timeout,
            start_time,
            event_buffer}
    }
    
    fn run(&mut self, ev: InputEvent) -> bool {
        match self.state {
            State::IDLE => {self.state_idle(ev)}
            State::DECIDE => {self.state_decide(ev)}
            State::SHIFT => {self.state_shift(ev)}
        }
    }
    
    fn state_idle(&mut self, ev: InputEvent) -> bool {
        let ev_kind = ev.kind();
        let ev_code = ev.code();
        let ev_value = ev.value();
        let p = self.conn.with_proxy(
            DBUS_IFACE_NAME,
            DBUS_PATH,
            Duration::from_millis(1000));
        let paused_refarg = p.get::<Box<dyn arg::RefArg>>(
            DBUS_IFACE_NAME,
             DBUS_PROP_NAME
            ).unwrap();
        let paused: bool = *arg::cast::<bool>(&paused_refarg).unwrap();
        if ev_kind == InputEventKind::Key(self.pause_key) && ev_value == KeyState::DOWN as i32 {
            p.set(DBUS_IFACE_NAME,
                DBUS_PROP_NAME,
                !paused).unwrap();
            return true;
        } else if ev_kind == InputEventKind::Key(self.fn_key) && !paused {
            self.start_time = Instant::now();
            self.state = State::DECIDE;
            return true;
        }
        
        send_key_i32(&mut self.virt_dev, ev_code, ev_value);
        false
    }
    
    fn state_decide(&mut self, ev: InputEvent) -> bool {
        let current_time = Instant::now();
        if current_time.duration_since(self.start_time) >= self.timeout {
            // Send all buffered key events as down then up
            for i in &self.event_buffer {
                let mut code = *i;
                if self.keymap.contains_key(&code) {
                    code = self.keymap[&code];
                }
                send_key_down(&mut self.virt_dev, code);
                send_key_up(&mut self.virt_dev, code);
            }
            self.event_buffer.clear();
            self.state = State::SHIFT;
            return true;
        } else {
            match ev.value().into() {
                KeyState::DOWN => { 
                    // add to event buffer
                    self.event_buffer.push(ev.code());
                }
                KeyState::UP => {
                    let mut code = ev.code();
                    if ev.kind() == InputEventKind::Key(self.fn_key) {
                        send_key_down(&mut self.virt_dev, code);
                        send_key_up(&mut self.virt_dev, code);
                        // Send all buffered key events as down
                        for i in &self.event_buffer {
                            send_key_down(&mut self.virt_dev, *i);
                        }
                        self.event_buffer.clear();
                        self.state = State::IDLE;
                        println!("decide -> idle");
                        return true;
                    } else if self.event_buffer.contains(&code) {
                        // remove ev from buffer
                        self.event_buffer.retain(|c| c != &code);
                        if self.keymap.contains_key(&code) {
                            code = self.keymap[&code];
                        }
                        
                        send_key_down(&mut self.virt_dev, code);
                        send_key_up(&mut self.virt_dev, code);
                        self.state = State::SHIFT;
                        println!("decide -> shift :: key press");
                        return true;
                    } else {
                        // key was pressed before fn_key
                        send_key_i32(&mut self.virt_dev, ev.code(), ev.value());
                    }
                }
                _ => {}
            }
        }
        
        false
    }
    
    fn state_shift(&mut self, ev: InputEvent) -> bool {
        if ev.kind() == InputEventKind::Key(self.fn_key) {
            if ev.value() == KeyState::UP as i32 {
                // Send all buffered key events as up
                for i in &self.event_buffer {
                    send_key_up(&mut self.virt_dev, *i);
                }
                self.event_buffer.clear();
                self.state = State::IDLE;
            }
        }

        if self.keymap.contains_key(&ev.code()) {
            let mapped_code = self.keymap[&ev.code()];
            
            match ev.value().into() {
                KeyState::UP => {
                    // remove ev from buffer
                    self.event_buffer.retain(|c| c != &mapped_code);
                }
                KeyState::DOWN => {
                    self.event_buffer.push(mapped_code);
                }
                _ => {}
            }

            send_key_i32(&mut self.virt_dev, mapped_code, ev.value());
        } else {
            send_key_i32(&mut self.virt_dev, ev.code(), ev.value());
        }
        
        false
    }
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    // setup
    let config = get_config();
    let mut dev = get_device(config.device_name.to_string()).expect("Invalid input device");
    let virt_dev = evdev::uinput::VirtualDeviceBuilder::new()?
        .name("spacefn-kbd")
        .with_keys(dev.supported_keys().unwrap())?
        .build()
        .unwrap();
        
    register_dbus_iface()?;
    
    let conn = Connection::new_session()?;
    let mut state_machine = StateMachine::new(conn, virt_dev, config);
    let _ = dev.grab();
    loop {
        match dev.fetch_events() {
            Ok(iterator) => {
                for ev in iterator {
                    if ev.code() == 0 || ev.event_type() != evdev::EventType::KEY {
                        continue;
                    }
                    
                    if state_machine.run(ev) {
                        break;
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