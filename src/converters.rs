
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};


use crate::parsers::sw_forecast_parser::*;


#[derive(Serialize, Debug, Clone)]
struct KpIndex {
    time_tag: String,
    kp: f32,
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


pub fn converter_kp(raw_text: String) -> Result::<String, String> {
    let raw_data: Vec<Vec<String>> = serde_json::from_str(raw_text.as_str())
        .map_err(|e| format!("deserilisation error: {e}"))?;

    let num_elements = 7;   // FIXME

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
                time_tag: convert_datetime(time_tag, "%Y-%m-%d %H:%M:%S%.3f", 3)?,
                kp: kp.parse().unwrap_or(0.0),
            });
        } else {
            return Err(format!("error during parsing data"));
        }
    }

    serde_json::to_string(&kp_data).map_err(|e| format!("serilisation error: {e}"))
}

pub fn converter_kp_inst(raw_text: String) -> Result::<String, String> {
    let raw_data: Vec<KpInst> = serde_json::from_str(raw_text.as_str())
        .map_err(|e| format!("deserilisation error: {e}"))?;

    // get only the most recent (last) element
    let last_element = raw_data.last().ok_or_else(|| format!("got no data"))?;

    let current_kp = KpIndex {
        time_tag: convert_datetime(&last_element.time_tag, "%Y-%m-%dT%H:%M:%S%Z", 0)?,
        kp: last_element.kp_index,
    };

    serde_json::to_string(&current_kp).map_err(|e| format!("serilisation error: {e}"))
}

pub fn converter_flux(raw_text: String) -> Result::<String, String> {
    let raw_data: Vec<ProtonFlux> = serde_json::from_str(raw_text.as_str())
        .map_err(|e| format!("deserilisation error: {e}"))?;

    let num_records = 2;    // FIXME: make custom struct with const field

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
        flux_gt500mev: 0.0
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
            mqtt_record.time_tag = convert_datetime(item.time_tag.as_str(), "%Y-%m-%dT%H:%M:%S%Z", 0)?;
            flux_records.push(mqtt_record.clone());
        }
    }

    serde_json::to_string(&flux_records).map_err(|e| format!("serilisation error: {e}"))
}

pub fn converter_sw_forecast(raw_text: String) -> Result::<String, String> {
    let sw_data = parse_sw_forecast(raw_text.as_str())?;

    for kp_data in &sw_data.kp {
        println!("Date: {}, Time end: {}, Kp: {}", kp_data.date, kp_data.hour, kp_data.value);
    }
    for srs_data in &sw_data.srs {
        println!("Date: {}, S1: {}, S2: {}, S3: {}, S4: {}, S5: {}", srs_data.date, srs_data.s1, srs_data.s2, srs_data.s3, srs_data.s4, srs_data.s5);
    }
    for rb_data in &sw_data.rb {
        println!("Date: {}, R1: {}, R2: {}, R3: {}, R4: {}, R5: {}", rb_data.date, rb_data.s1, rb_data.s2, rb_data.s3, rb_data.s4, rb_data.s5);
    }

    serde_json::to_string(&sw_data).map_err(|e| format!("serilisation error: {e}"))
}

pub fn convert_datetime(input: &str, in_format: &str, offset_hours: i64) -> Result::<String, String> {
    let mut datetime = NaiveDateTime::parse_from_str(input, in_format)
        .map_err(|e| format!("parsing datetime string error: {e}"))?;
    datetime += chrono::Duration::hours(offset_hours);
    Ok(datetime.format("%H:%M %d-%m-%Y").to_string())
}
