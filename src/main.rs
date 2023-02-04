use crc32fast::Hasher;
use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use lazy_static::lazy_static;
use prometheus::{opts, register_gauge_vec};
use prometheus::{Encoder, GaugeVec, TextEncoder};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

lazy_static! {
    static ref ROOM_TEMPERATURE: GaugeVec =
        register_gauge_vec!("room_temperature", "Room temperature in degrees", &["unit"]).unwrap();
    static ref HUMIDITY: GaugeVec =
        register_gauge_vec!("humidity", "Humidity in percent", &["unit"]).unwrap();
    static ref PRESSURE: GaugeVec =
        register_gauge_vec!("air_pressure", "Pressure in kPa", &["unit"]).unwrap();
    static ref BATTERY: GaugeVec =
        register_gauge_vec!("sensor_battery", "Battery Volts", &["unit"]).unwrap();
}

fn mac_string(mac: &[u8; 6]) -> String {
    format!(
        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

async fn got_message(msg: &[u8], sensors: &Mutex<HashMap<[u8; 6], Instant>>) {
    if msg.len() < 4 {
        eprintln!("too short");
        return;
    }
    let sum_bytes: [u8; 4] = msg[..4].try_into().unwrap();
    let got_sum = u32::from_be_bytes(sum_bytes);
    let mut h = Hasher::new();
    h.update(&msg[4..]);
    let want_sum = h.finalize();
    if got_sum != want_sum {
        eprintln!("CRC32 mismatch");
        return;
    }
    if msg.len() >= 30 && msg[4] == 0x99 && msg[5] == 0x04 && msg[6] == 5 {
        // https://github.com/ruuvi/ruuvi-sensor-protocols/blob/master/dataformat_05.md
        let mac: [u8; 6] = msg[24..30].try_into().unwrap();
        if mac == [0xff; 6] {
            eprintln!("missing MAC");
            return;
        }

        let expiry = Instant::now() + Duration::from_secs(300);
        sensors
            .lock()
            .await
            .entry(mac)
            .and_modify(|e| *e = expiry)
            .or_insert(expiry);

        let mac_s = mac_string(&mac);
        let labels = &[mac_s.as_str()];
        let temp_raw = i16::from_be_bytes(msg[7..9].try_into().unwrap());
        if temp_raw == i16::MIN {
            ROOM_TEMPERATURE.remove_label_values(labels).ok();
        } else {
            ROOM_TEMPERATURE
                .with_label_values(labels)
                .set(temp_raw as f64 * 0.005);
        }
        let humidity_raw = u16::from_be_bytes(msg[9..11].try_into().unwrap());
        if humidity_raw == u16::MAX {
            HUMIDITY.remove_label_values(labels).ok();
        } else {
            HUMIDITY
                .with_label_values(labels)
                .set(humidity_raw as f64 * 0.0025);
        }
        let pressure_raw = u16::from_be_bytes(msg[11..13].try_into().unwrap());
        if pressure_raw == u16::MAX {
            PRESSURE.remove_label_values(labels).ok();
        } else {
            PRESSURE
                .with_label_values(labels)
                .set(pressure_raw as f64 / 1000.0 + 50.0);
        }
        let power_raw = u16::from_be_bytes(msg[19..21].try_into().unwrap());
        if power_raw >> 5 == 2047 {
            BATTERY.remove_label_values(labels).ok();
        } else {
            BATTERY
                .with_label_values(labels)
                .set((power_raw >> 5) as f64 / 1000.0 + 1.6);
        }
    }
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    let encoder = TextEncoder::new();

    let metric_families = prometheus::gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();

    let response = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap();

    Ok(response)
}

fn is_arduino(prefix: &PathBuf) -> bool {
    match fs::read_to_string(prefix.join("device/../idVendor")) {
        Ok(contents) if contents == "2341\n" => (),
        _ => {
            return false;
        }
    };
    match fs::read_to_string(prefix.join("device/../idProduct")) {
        Ok(contents) if contents == "8054\n" => true,
        _ => false,
    }
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        0x30..=0x39 => Some(b - 0x30),
        0x41..=0x4f => Some(b - 55),
        0x61..=0x6f => Some(b - 87),
        _ => None,
    }
}

enum ReadState {
    Interstitial,
    Open1,
    Open2,
    Nibble1,
    Nibble2,
    Close1,
    Close2,
}

async fn arduino_bridge(path: &Path, sensors: &Mutex<HashMap<[u8; 6], Instant>>) -> std::io::Result<()> {
    let mut input = File::open(path).await?;
    let mut msg = Vec::new();
    let mut n = 0;
    let mut state = ReadState::Interstitial;
    loop {
        let mut buffer = [0u8; 1024];
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        for b in buffer[..count].iter() {
            match state {
                ReadState::Interstitial => {
                    if *b == 123 {
                        state = ReadState::Open1;
                    }
                }
                ReadState::Open1 => {
                    state = if *b == 123 {
                        ReadState::Open2
                    } else {
                        ReadState::Interstitial
                    }
                }
                ReadState::Open2 => {
                    state = if *b == 123 {
                        msg = Vec::new();
                        ReadState::Nibble1
                    } else {
                        ReadState::Interstitial
                    }
                }
                ReadState::Nibble1 => {
                    state = if let Some(nn) = nibble(*b) {
                        n = nn;
                        ReadState::Nibble2
                    } else if *b == 125 {
                        ReadState::Close1
                    } else {
                        ReadState::Interstitial
                    }
                }
                ReadState::Nibble2 => {
                    state = if let Some(nn) = nibble(*b) {
                        msg.push(n << 4 | nn);
                        if msg.len() < 500 {
                            ReadState::Nibble1
                        } else {
                            ReadState::Interstitial // too long
                        }
                    } else {
                        ReadState::Interstitial
                    }
                }
                ReadState::Close1 => {
                    state = if *b == 125 {
                        ReadState::Close2
                    } else {
                        ReadState::Interstitial
                    }
                }
                ReadState::Close2 => {
                    if *b == 125 {
                        got_message(&msg, sensors).await;
                    }
                    state = ReadState::Interstitial;
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 2 {
        eprintln!(
            "Usage: {} export-listen",
            args[0].to_string_lossy()
        );
        std::process::exit(3);
    }
    let metric_addr: SocketAddr = args[1].to_string_lossy().into_owned().parse()?;

    let serve_future = Server::bind(&metric_addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(serve_req))
    }));

    let sensors = Arc::new(Mutex::new(HashMap::<[u8; 6], Instant>::new()));

    let sensors_update = sensors.clone();
    tokio::spawn(async move {
        loop {
            let maybe_ttyname = match fs::read_dir("/sys/class/tty") {
                Ok(r) => r
                    .filter_map(|e| {
                        match e {
                            Ok(entry) => {
                                if is_arduino(&entry.path()) {
                                    Some(entry.file_name())
                                } else {
                                    None
                                }
                            }
                            Err(_) => None
                        }
                    })
                    .nth(0),
                Err(e) => {
                    eprintln!("Scanning /sys/class/tty failed: {}", e);
                    None
                }
            };
            if let Some(ttyname) = maybe_ttyname {
                let path = Path::new("/dev").join(ttyname);
                println!("Using {}...", path.display());
                if let Err(e) = arduino_bridge(&path, &sensors_update).await {
                    eprintln!("Error reading from Arduino: {}", e);
                }
            } else {
                eprintln!("Found no device to read from.");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let now = Instant::now();
            let expired: Vec<_> = sensors
                .lock()
                .await
                .iter()
                .filter(|(_, expiry)| **expiry < now)
                .map(|(k, _)| *k)
                .collect();
            for mac in expired {
                let mac_s = mac_string(&mac);
                let labels = &[mac_s.as_str()];
                ROOM_TEMPERATURE.remove_label_values(labels).ok();
                HUMIDITY.remove_label_values(labels).ok();
                PRESSURE.remove_label_values(labels).ok();
                BATTERY.remove_label_values(labels).ok();
            }
            sensors.lock().await.retain(|_, &mut expiry| expiry >= now);
        }
    });

    if let Err(err) = serve_future.await {
        eprintln!("server error: {}", err);
    }
    Ok(())
}
