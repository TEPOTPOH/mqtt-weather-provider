extern crate chrono;
extern crate rumqttc;
extern crate envconfig;

use chrono::{NaiveDateTime};
use reqwest::Error;
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};
use std::sync::{Arc, Mutex};
use envconfig::Envconfig;
use rumqttc::{MqttOptions, Client, QoS};

pub mod parsers;

use parsers::sw_forecast_parser::*;

#[derive(Envconfig, Debug)]
struct Config {
    #[envconfig(from = "MQTT_BROKER_HOST", default = "localhost")]
    pub mqtt_host: String,

    #[envconfig(from = "MQTT_BROKER_PORT", default = "1883")]
    pub mqtt_port: u16,

    #[envconfig(from = "MQTT_BROKER_KEEP_ALIVE", default = "5")]
    pub mqtt_keep_alive: u16,

    #[envconfig(from = "MQTT_BROKER_BASE_TOPIC", default = "homeassistant/sensor")]
    pub mqtt_base_topic: String,

    #[envconfig(from = "MQTT_DEVICE_NAME", default = "unknown")]
    pub mqtt_device_name: String,

    #[envconfig(from = "KP_RELEASE_INTERVAL_S", default = "600")]   // 10 min
    pub kp_release_interval_s: u16,

    #[envconfig(from = "KP_INST_INTERVAL_S", default = "300")]     // 5 min
    pub kp_inst_interval_s: u16,
}

fn convert_datetime(input: &str, in_format: &str, offset_hours: i64) -> String {
    let mut datetime = NaiveDateTime::parse_from_str(input, in_format).expect("Failed to parse datetime");
    datetime += chrono::Duration::hours(offset_hours);
    return datetime.format("%H:%M %d-%m-%Y").to_string();
}

#[derive(Serialize, Debug, Clone)]
struct KpIndex {
    time_tag: String,
    kp: f32,
}

async fn fetch_kp_release(num_elements: usize) -> Result<Vec<KpIndex>, String> {
    let url = "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json";
    // TODO: move this fetcher to another function
    let fetch_data = async {
        reqwest::get(url).await?    // make GET request
            .error_for_status()?    // handling HTTP status
            .json::<Vec<Vec<String>>>().await.map(|data| Ok(data))?    // Deserialize JSON and wrap data into Result type
    };
    // Block of code "fetch_data" returns type Result<T, Error> and it should be converted to Result<T, String>
    let raw_data = fetch_data.await.map_err(|e: Error| format!("reqwest error: {e}"))?;

    // skip header
    let data_without_header = &raw_data[1..];

    // determine initial index for slice
    let start_index = if data_without_header.len() > num_elements {
        data_without_header.len() - num_elements
    } else {
        0
    };

    // make slice with needed number of last elements
    let required_data = &data_without_header[start_index..];

    // move data to structs
    let mut kp_data: Vec<KpIndex> = Vec::with_capacity(num_elements);
    for item in required_data.iter() {
        if let [time_tag, kp, ..] = &item[..] {
            kp_data.push(KpIndex {
                // add offset +3H to provide intervals's end timestamp insted of start timestamp
                time_tag: convert_datetime(time_tag, "%Y-%m-%d %H:%M:%S%.3f", 3),
                kp: kp.parse().unwrap_or(0.0),
            });
        } else {
            return Err(format!("error during parsing data"));
        }
    }

    return Ok(kp_data);
}

async fn update_kp_release(num_elements: usize, shared_kp_data: Arc<Mutex<Vec<KpIndex>>>) -> Result<(), String> {
    let kp_data = match fetch_kp_release(num_elements).await {
        Ok(kp_data) => kp_data,
        Err(e) => {
            return Err(format!("Ошибка при получении данных из fetch_kp_release: {}", e));
        },
    };
    let mut data = shared_kp_data.lock().unwrap();
    data.clear();
    data.extend(kp_data);
    println!("1 Fetched new Kp data");
    return Ok(());
}

#[derive(Deserialize, Debug, Clone)]
struct KpInst {
    time_tag: String,
    kp_index: f32,
    #[serde(skip_deserializing)]
    estimated_kp: f32,
    #[serde(skip_deserializing)]
    kp: String,
}

async fn fetch_kp_instant() -> Result<KpIndex, String> {
    let url = "https://services.swpc.noaa.gov/json/planetary_k_index_1m.json";

    // TODO: move this fetcher to another function
    let fetch_data = async {
        reqwest::get(url).await?    // make GET request
            .error_for_status()?    // handling HTTP status
            .json::<Vec<KpInst>>().await.map(|data| Ok(data))?    // Deserialize JSON and wrap data into Result type
    };
    // Block of code "fetch_data" returns type Result<T, Error> and it should be converted to Result<T, String>
    let raw_data = fetch_data.await.map_err(|e: Error| format!("reqwest error: {e}"))?;

    // get only the most recent (last) element
    let last_element = raw_data.last().unwrap();  // TODO: (|e| format!("no valid data: {e}"))?;

    let current_kp = KpIndex {
        time_tag: convert_datetime(&last_element.time_tag, "%Y-%m-%dT%H:%M:%S%Z", 0),
        kp: last_element.kp_index,
    };
    return Ok(current_kp);
}

async fn update_kp_instant(shared_kp_data: Arc<Mutex<KpIndex>>) -> Result<(), String> {
    let last_kp_inst = match fetch_kp_instant().await {
        Ok(data) => data,
        Err(e) => {
            return Err(format!("Ошибка при получении данных из fetch_kp_instant: {}", e));
        },
    };
    let mut data = shared_kp_data.lock().unwrap();
    *data = last_kp_inst;
    return Ok(());
}

fn make_full_topic(sensor_name: &str, config: &Config) -> String {
    let full_topic = config.mqtt_base_topic.clone() + "/" + &config.mqtt_device_name + "_" + sensor_name + "/state";
    return full_topic;
}

async fn send_to_broker(client: Arc<Mutex<Client>>, topic: String, payload: String) -> Result<(), String> {
    println!("publish topic {} with payload: ", topic);
    println!("{:#}", payload);
    let mut mut_client = client.lock().unwrap();
    match mut_client.publish(topic, QoS::AtLeastOnce, false, payload.as_bytes()) {
        Ok(_) => {},
        Err(e) => {
            return Err(format!("MQTT publish error: {}", e));
        },
    };
    return Ok(());
}

async fn send_kp(mut kp_release: Vec<KpIndex>, last_kp_inst: KpIndex, config: &Config, client: Arc<Mutex<Client>>) -> Result<(), String> {
    kp_release.push(last_kp_inst);
    for entry in &kp_release {
        println!("Time: {}, Kp: {}", entry.time_tag, entry.kp);
    }
    let topic = make_full_topic("nasa_kp", &config);
    send_to_broker(client, topic, serde_json::to_string(&kp_release).unwrap()).await?;
    return Ok(());
}

async fn init_mqtt(config: &Config) -> Result<Arc<Mutex<Client>>, String> {
    let mut mqttoptions = MqttOptions::new("weather-provider", &config.mqtt_host, config.mqtt_port);
    mqttoptions.set_keep_alive(Duration::from_secs(config.mqtt_keep_alive.into()));
    println!("Connecting to MQTT broker...");
    let (mut client, mut connection) = Client::new(mqttoptions, 10);

    let mut client_ref = Arc::new(Mutex::new(client));

    println!("Spawn Connection handler thread");
    // Connection handler thread
    tokio::task::spawn_blocking( move || {
        println!("Connection handler thread spawned");
        loop {
            // The `EventLoop`/`Connection` must be regularly polled(`.next()` in case of `Connection`) in order
            // to send, receive and process packets from the broker, i.e. move ahead.
            for (_, notification) in connection.iter().enumerate() {
                println!("Notification = {:?}", notification);
            }
        }
    });
    return Ok(client_ref.clone());
}

#[tokio::main]
async fn main() {
    let http_error_timeout = Duration::from_secs(60);
    let mqtt_error_timeout = Duration::from_secs(30);
    let num_elements = 7;

    let config = Config::init_from_env().unwrap();

    println!("Using config:\n{:?}", config);

    let kp_data_init = KpIndex {
        time_tag: "00:00 01-01-2024".to_string(),
        kp: 0.0,
    };
    let kp_release = Arc::new(Mutex::new(Vec::from(vec![
        kp_data_init.clone();
        num_elements
        ])));
    let kp_inst = Arc::new(Mutex::new(kp_data_init));
    let mut kp_release_data_interval = tokio::time::interval(Duration::from_secs(config.kp_release_interval_s.into()));
    let mut instant_data_interval = tokio::time::interval(Duration::from_secs(config.kp_inst_interval_s.into()));
    
    let mut client_ref = init_mqtt(&config).await.unwrap();

    // TODO: waiting for connection

    // TODO: limit max time for each async function
    
    // initial fetch and send data
    let initial_tasks = async {
        update_kp_release(num_elements, kp_release.clone()).await.ok();
        update_kp_instant(kp_inst.clone()).await.ok();
        send_kp(kp_release.lock().unwrap().clone(),
                kp_inst.lock().unwrap().clone(),
                &config,
                client_ref.clone()).await.ok();
        let flux_data = fetch_flux(2).await.unwrap_or_default();
        send_flux(flux_data,
                  &config,
                  client_ref.clone()).await.ok();
        let swf = fetch_space_forecast().await.unwrap_or_default();
        send_sw_forecast(swf, &config, client_ref.clone()).await.ok();
    };
    initial_tasks.await;

    loop {
        tokio::select! {
            _ = kp_release_data_interval.tick() => {
                match update_kp_release(num_elements, kp_release.clone()).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("{}", e);
                        println!("Tring extra timeout {:?} after connection error", http_error_timeout);
                        sleep(http_error_timeout).await;
                    }
                };
                // sw forecast
                let swf = match fetch_space_forecast().await {
                    Ok(data) => data,
                    Err(e) => {
                        println!("Ошибка при получении данных из fetch_space_forecast: {}", e);
                        println!("Tring extra timeout {:?} after connection error", http_error_timeout);
                        sleep(http_error_timeout).await;
                        break;
                    },
                };
                match send_sw_forecast(swf, &config, client_ref.clone()).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Send SW forecast data error: {}", e);
                        println!("Tring extra timeout {:?} after connection error", mqtt_error_timeout);
                        sleep(mqtt_error_timeout).await;
                    },
                };
            },
            _ = instant_data_interval.tick() => {
                // Kp
                match update_kp_instant(kp_inst.clone()).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("{}", e);
                        println!("Tring extra timeout {:?} after connection error", http_error_timeout);
                        sleep(http_error_timeout).await;
                    }
                };
                match send_kp(kp_release.lock().unwrap().clone(),
                              kp_inst.lock().unwrap().clone(),
                              &config,
                              client_ref.clone()).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Send Kp data error: {}", e);
                        println!("Tring extra timeout {:?} after connection error", mqtt_error_timeout);
                        sleep(mqtt_error_timeout).await;
                    },
                };
                // Fluxs
                let flux_data = match fetch_flux(2).await {
                    Ok(flux_data) => flux_data,
                    Err(e) => {
                        println!("Ошибка при получении данных из fetch_flux: {}", e);
                        println!("Tring extra timeout {:?} after connection error", http_error_timeout);
                        sleep(http_error_timeout).await;
                        break;
                    },
                };
                match send_flux(flux_data,
                               &config,
                               client_ref.clone()).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Send Flux data error: {}", e);
                        println!("Tring extra timeout {:?} after connection error", mqtt_error_timeout);
                        sleep(mqtt_error_timeout).await;
                    },
                };
            },
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct ProtonFlux {
    time_tag: String,
    #[serde(skip_deserializing)]
    satellite: u8,
    flux: f32,
    energy: String,
}

#[derive(Serialize, Debug, Clone)]
struct ProtonFluxMQTT {
    time_tag: String,
    flux_gt10mev: f32,
    flux_gt50mev: f32,
    flux_gt100mev: f32,
    flux_gt500mev: f32,
}

async fn fetch_flux(num_records: usize) -> Result<Vec<ProtonFluxMQTT>, String> {
    let url = "https://services.swpc.noaa.gov/json/goes/primary/integral-protons-plot-6-hour.json";
    // TODO: move this fetcher to another function
    let fetch_data = async {
        reqwest::get(url).await?    // make GET request
            .error_for_status()?    // handling HTTP status
            .json::<Vec<ProtonFlux>>().await.map(|data| Ok(data))?    // Deserialize JSON and wrap data into Result type
    };
    // Block of code "fetch_data" returns type Result<T, Error> and it should be converted to Result<T, String>
    let raw_data = fetch_data.await.map_err(|e: Error| format!("reqwest error: {e}"))?;

    // determine initial index for slice
    let num_elements = num_records * 4;
    let start_index = if raw_data.len() > num_elements {
        raw_data.len() - num_elements
    } else {
        0
    };

    // make slice with needed number of last elements
    let required_data = &raw_data[start_index..];

    // move data to structs
    let mut flux_records: Vec<ProtonFluxMQTT> = Vec::with_capacity(num_records);
    let mut mqtt_record = ProtonFluxMQTT {
        time_tag: "".to_string(),
        flux_gt10mev: 0.0,
        flux_gt100mev: 0.0,
        flux_gt50mev: 0.0,
        flux_gt500mev:0.0
    };
    for item in required_data.iter() {
        let flux_f32 = item.flux;
        if item.energy == ">=10 MeV" {
            mqtt_record.flux_gt10mev = flux_f32;
        } else if item.energy == ">=100 MeV" {
            mqtt_record.flux_gt100mev = flux_f32;
        } else if item.energy == ">=50 MeV" {
            mqtt_record.flux_gt50mev = flux_f32;
        } else if item.energy == ">=500 MeV" {
            mqtt_record.flux_gt500mev = flux_f32;
            mqtt_record.time_tag = convert_datetime(item.time_tag.as_str(), "%Y-%m-%dT%H:%M:%S%Z", 0);
            flux_records.push(mqtt_record.clone());
        }
    }
    return Ok(flux_records);
}

async fn send_flux(flux_data: Vec<ProtonFluxMQTT>, config: &Config, client: Arc<Mutex<Client>>) -> Result<(), String> {
    for entry in &flux_data {
        println!("Time: {}, flux >=10Mev: {}", entry.time_tag, entry.flux_gt10mev);
    }
    let topic = make_full_topic("nasa_flux", &config);
    send_to_broker(client, topic, serde_json::to_string(&flux_data).unwrap()).await?;
    return Ok(());
}

async fn fetch_space_forecast() -> Result<SWForecast, String> {
    let url = "https://services.swpc.noaa.gov/text/3-day-forecast.txt";
    let fetch_data = async {
        reqwest::get(url).await?    // make GET request
            .error_for_status()?    // handling HTTP status
            .text().await.map(|data| Ok(data))?    // Get text and wrap it into Result type
    };
    // Block of code "fetch_data" returns type Result<T, Error> and it should be converted to Result<T, String>
    let raw_data = fetch_data.await.map_err(|e: Error| format!("reqwest error: {e}"))?;

    let sw_data = parse_sw_forecast(raw_data.as_str())?;
    for kp_data in &sw_data.kp {
        println!("Date: {}, Time end: {}, Kp: {}", kp_data.date, kp_data.hour, kp_data.value);
    }
    for srs_data in &sw_data.srs {
        println!("Date: {}, S1: {}, S2: {}, S3: {}, S4: {}, S5: {}", srs_data.date, srs_data.s1, srs_data.s2, srs_data.s3, srs_data.s4, srs_data.s5);
    }
    for rb_data in &sw_data.rb {
        println!("Date: {}, R1: {}, R2: {}, R3: {}, R4: {}, R5: {}", rb_data.date, rb_data.s1, rb_data.s2, rb_data.s3, rb_data.s4, rb_data.s5);
    }

    return Ok(sw_data);
}

async fn send_sw_forecast(data: SWForecast, config: &Config, client: Arc<Mutex<Client>>) -> Result<(), String> {
    let topic = make_full_topic("nasa_sw_forecast", &config);
    send_to_broker(client, topic, serde_json::to_string(&data).unwrap()).await?;
    return Ok(());
}