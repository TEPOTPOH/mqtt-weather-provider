extern crate chrono;
extern crate rumqttc;
extern crate envconfig;

pub mod parsers;
pub mod converters;

use reqwest::Error;
use tokio::task;
use tokio::time::{Duration, interval};
use std::sync::{Arc, Mutex};
use envconfig::Envconfig;
use rumqttc::{MqttOptions, Client, QoS};
use converters::*;


type TconvertFn = fn(String) -> Result::<String, String>;

#[derive(Clone)]
struct TWeatherSource {
    source_url: &'static str,
    mqtt_topic_name: &'static str,
    request_interval_s: u16,
    convert: TconvertFn
}

struct TWeatherProvider {
    // sources: Vec<TWeatherSource>,
    transmitter: TMQTTransmitter,
}

impl TWeatherProvider {
    async fn provide(&self, source: &TWeatherSource) -> Result::<(), String> {
        let raw_data = self.load_text(source).await.map_err(|e: Error| format!("HTTP reqwest error: {e}"))?;
        let payload = (source.convert)(raw_data)?;
        self.send(source, payload).await
    }
    async fn send(&self, source: &TWeatherSource, payload: String) -> Result::<(), String> {
        self.transmitter.send_to_broker(source.mqtt_topic_name, payload).await
    }
    async fn load_text(&self, source: &TWeatherSource) -> Result::<String, Error> {
        reqwest::get(source.source_url).await?    // make GET request
                .error_for_status()?    // handling HTTP status
                .text().await
    }
}


struct TMQTTSettings {
    name: &'static str,
    config: Arc<Config>,
}

struct TMQTTransmitter {
    settings: TMQTTSettings,
    client: Arc<Mutex<Client>>,
}

impl TMQTTransmitter {
    fn init(settings: TMQTTSettings) -> Result<Self, String> {
        let mut mqttoptions = MqttOptions::new(settings.name, &settings.config.mqtt_host, settings.config.mqtt_port);
        mqttoptions.set_keep_alive(Duration::from_secs(settings.config.mqtt_keep_alive.into()));
        println!("Connecting to MQTT broker...");
        let (client, mut connection) = Client::new(mqttoptions, 10);

        let transmitter = Self { settings: settings, client: Arc::new(Mutex::new(client)) };

        println!("Spawn Connection handler thread");
        // Connection handler thread
        task::spawn_blocking( move || {
            println!("Connection handler thread spawned");
            loop {
                // The `EventLoop`/`Connection` must be regularly polled(`.next()` in case of `Connection`) in order
                // to send, receive and process packets from the broker, i.e. move ahead.
                for (_, notification) in connection.iter().enumerate() {
                    println!("Notification = {:?}", notification);
                }
            }
        });
        return Ok(transmitter);
    }

    async fn send_to_broker(&self, topic: &str, payload: String) -> Result<(), String> {
        let full_topic = Self::make_full_topic(topic, &self.settings.config);
        println!("MQTT publish topic {} with payload: ", full_topic);
        println!("{:#}", payload);
        let mut mut_client = self.client.lock().expect("Error when locking MQTT client mutex");
        mut_client.publish(full_topic, QoS::AtLeastOnce, false, payload.as_bytes())
            .map_err(|e| format!("MQTT publish error: {e}"))
    }

    fn make_full_topic(sensor_name: &str, config: &Config) -> String {
        let full_topic = config.mqtt_base_topic.clone() + "/" + &config.mqtt_device_name + "_" + sensor_name + "/state";
        return full_topic;
    }
}


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


#[tokio::main]
async fn main() {
    // stack, immutable, all time live, multithreading read access
    let config = Config::init_from_env().unwrap();

    println!("Using config:\n{:?}", config);

    // stack/data, immutable, all time live, multithreading read access
    let weather_sources = [
        TWeatherSource { source_url: "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json",
                         mqtt_topic_name: "noaa_kp",
                         request_interval_s: config.kp_release_interval_s,
                         convert: converter_kp
                        },
        TWeatherSource { source_url: "https://services.swpc.noaa.gov/json/planetary_k_index_1m.json",
                         mqtt_topic_name: "noaa_kp_inst",
                         request_interval_s: config.kp_inst_interval_s,
                         convert: converter_kp_inst
                       },
        TWeatherSource { source_url: "https://services.swpc.noaa.gov/json/goes/primary/integral-protons-plot-6-hour.json",
                         mqtt_topic_name: "noaa_flux",
                         request_interval_s: config.kp_inst_interval_s,
                         convert: converter_flux
                       },
        TWeatherSource { source_url: "https://services.swpc.noaa.gov/text/3-day-forecast.txt",
                         mqtt_topic_name: "noaa_sw_forecast",
                         request_interval_s: config.kp_release_interval_s,
                         convert: converter_sw_forecast
                       },
    ];

    let mqtt = TMQTTransmitter::init(TMQTTSettings {
                                        name: "weather-provider",
                                        config: Arc::new(config)
                                    }).unwrap();

    // TODO: waiting for connection

    let wprovider = TWeatherProvider {
        transmitter: mqtt
    };

    // let repeater = TRepeater {
    //     provider: wprovider,
    //     sources: weather_sources.into()
    // }

    let wprovider_ref = Arc::new(wprovider);

    for source in weather_sources {
        let wprovider_ref2 = wprovider_ref.clone();
        let ws = source;
        let handler = task::spawn(async move {
            let mut interval = interval(Duration::from_secs(ws.request_interval_s.into()));
            loop {
                interval.tick().await;
                // TODO: limit max time for loading and sending
                let result = wprovider_ref2.provide(&ws).await;
                match result {
                    // TODO: error handling
                    Err(e) => println!("Error during providing source {}: {}", ws.mqtt_topic_name, e),
                    Ok(_) => {}
                }
            }
        });
        tokio::try_join!(handler).unwrap();
    }
}
