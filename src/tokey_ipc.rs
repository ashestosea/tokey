use dbus::blocking::Connection;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_crossroads::{Crossroads, IfaceBuilder};
use std::time::Duration;

const DBUS_IFACE_NAME: &str = "com.chronotab.tokey";
const DBUS_PATH: &str = "/";
const DBUS_PROP_NAME: &str = "Paused";

pub struct Messenger {
    conn: Connection
}

impl Messenger {
    pub fn new() -> Self {
        register_dbus_iface().expect("Cannot register dbus interface");
        
        Messenger { conn: Connection::new_session().expect("Cannot create dbus session") }
    }
    
    fn get_proxy(&self) -> dbus::blocking::Proxy<&Connection> {
        self.conn.with_proxy(DBUS_IFACE_NAME, DBUS_PATH, Duration::from_millis(1000))
    }
    
    pub fn set_paused(&self, paused: bool) {
        self.get_proxy().set(DBUS_IFACE_NAME, DBUS_PROP_NAME, !paused).unwrap();
    }
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
