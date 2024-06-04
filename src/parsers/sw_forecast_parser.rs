extern crate nom;
use nom::{
    bytes::complete::{tag, take_until},
    character::complete::{alpha1, alphanumeric1, digit1, line_ending, multispace1, not_line_ending, space0, space1},
    combinator::opt,
    multi::many1,
    number::complete::float,
    sequence::{delimited, preceded, tuple},
    Finish, IResult,
};
use serde::Serialize;
use std::str::FromStr;

#[derive(Serialize, Debug, Clone, Default)]
pub struct KPForecast {
    pub date: String,
    pub hour: u8,
    pub value: f32,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct SRSRBForecast {
    pub date: String,
    pub s1: u8,
    pub s2: u8,
    pub s3: u8,
    pub s4: u8,
    pub s5: u8,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct SWForecast {
    pub kp: Vec<KPForecast>,
    pub srs: Vec<SRSRBForecast>,
    pub rb: Vec<SRSRBForecast>,
}

// Парсер для заголовка с датами
fn parse_header<'a>(input: &'a str, header: &str) -> IResult<&'a str, Vec<String>> {
    let (input, _) = take_until(header)(input)?;
    let (input, _) = tuple((tag(header), multispace1))(input)?;
    let (input, dates_wyear) = not_line_ending(input)?;
    let year = " ".to_string() + dates_wyear.split(' ').last().unwrap();
    let (input, _) = line_ending(input)?;
    let (input, _) = line_ending(input)?;
    let (input, mut dates) = many1(preceded(space1, parse_date))(input)?;
    for date in &mut dates {
        *date += year.as_str();
    }
    let (input, _) = line_ending(input)?;
    Ok((input, dates))
}

fn parse_date(input: &str) -> IResult<&str, String> {
    let (input, month) = alpha1(input)?;
    let (input, _) = space1(input)?;
    let (input, day) = digit1(input)?;
    Ok((input, format!("{} {}", month, day)))
}

// Парсер для строк с временными интервалами и Kp значениями
fn parse_kp_row(input: &str) -> IResult<&str, (u8, u8, Vec<f32>)> {
    let (input, (time_range_start, time_range_end)) = parse_time_range(input)?;
    let (input, kps) = many1(preceded(space0, parse_kp_val))(input)?;
    let (input, _) = opt(multispace1)(input)?;
    let (input, _) = opt(line_ending)(input)?;
    Ok((input, (time_range_start, time_range_end, kps)))
}

fn parse_kp_val(input: &str) -> IResult<&str, f32> {
    let (input, kp_value) = float(input)?;
    let (input, _) = opt(space1)(input)?;
    let (input, _) = opt(delimited(tag("("), alphanumeric1, tag(")")))(input)?;
    Ok((input, kp_value))
}

// Парсер для временного интервала
fn parse_time_range(input: &str) -> IResult<&str, (u8, u8)> {
    let (input, start) = digit1(input)?;
    let (input, _) = tag("-")(input)?;
    let (input, end) = digit1(input)?;
    let (input, _) = tag("UT")(input)?;
    Ok((input, (u8::from_str(start).unwrap(), u8::from_str(end).unwrap())))
}

fn parse_kp_forecast(input: &str) -> IResult<&str, Vec<KPForecast>> {
    let (input, dates) = parse_header(input, "NOAA Kp index breakdown").unwrap();
    let (input, rows) = many1(parse_kp_row)(input)?;

    let mut results = Vec::new();
    for (_, time_range_end, kps) in rows {
        for (index, kp) in kps.into_iter().enumerate() {
            let date = &dates[index];
            results.push(KPForecast {
                date: date.clone(),
                hour: time_range_end,
                value: kp,
            });
        }
    }

    // sort
    results.sort_by(|kpf1, kpf2| kpf1.date.cmp(&kpf2.date));

    Ok((input, results))
}

fn parse_prcnt_val(input: &str) -> IResult<&str, u8> {
    let (input, value) = digit1(input)?;
    let (input, _) = tag("%")(input)?;
    Ok((input, u8::from_str(value).unwrap()))
}

fn parse_solar_rb_storms(input: &str, storm_type: char) -> IResult<&str, (u8, u8)> {
    let (input, (_, s_min_str)) = tuple((tag(storm_type.to_string().as_str()), digit1))(input)?;
    let s_min = u8::from_str(s_min_str).unwrap();
    let mut s_max = s_min + 1;
    if s_max > 5 {
        s_max = 5;
    };
    let (input, greater) = opt(tag(" or greater"))(input)?;
    let input = match greater {
        Some(_) => input,
        None => {
            let (input, _) = tag("-")(input)?;
            let (input, (_, max)) = tuple((tag(storm_type.to_string().as_str()), digit1))(input)?;
            s_max = u8::from_str(max).unwrap();
            input
        },
    };
    Ok((input, (s_min, s_max)))
}

// Парсер для строк с категорией шторма и значениями вероятности шторма
fn parse_srs_rb_row(input: &str, storm_type: char) -> IResult<&str, (u8, u8, Vec<u8>)> {
    let (input, (s_min, s_max)) = parse_solar_rb_storms(input, storm_type)?;
    let (input, values) = many1(preceded(space0, parse_prcnt_val))(input)?;
    let (input, _) = opt(multispace1)(input)?;
    let (input, _) = opt(line_ending)(input)?;
    Ok((input, (s_min, s_max, values)))
}

fn parse_srs_rb_forecast<'a>(input: &'a str, header_phrase: &str, storm_type: char) -> IResult<&'a str, Vec<SRSRBForecast>> {
    let (input, dates) = parse_header(input, header_phrase).unwrap();
    let (input, rows) = many1(|i| parse_srs_rb_row(i, storm_type))(input)?;

    let mut results: Vec<SRSRBForecast> = Vec::new();
    for (s_min, s_max, values) in rows {
        for (index, value) in values.into_iter().enumerate() {
            let date = &dates[index];
            // find in results record with same date and use it or create new one if it don't exists
            let srs = match results.iter_mut().rfind(|srs_val| &srs_val.date == date) {
                Some(val) => val,
                None => {
                    results.push(SRSRBForecast {
                        date: date.clone(),
                        s1: 0,
                        s2: 0,
                        s3: 0,
                        s4: 0,
                        s5: 0,
                    });
                    results.last_mut().unwrap()
                },
            };
            let srs_vec = [&mut srs.s1, &mut srs.s2, &mut srs.s3, &mut srs.s4, &mut srs.s5];
            assert!(s_min >= 1 && s_min <= 5);
            assert!(s_max >= 1 && s_max <= 5);
            for si in (s_min - 1)..s_max {
                *srs_vec[usize::from(si)] = value;
            }
        }
    }
    // sort
    results.sort_by(|elm1, elm2| elm1.date.cmp(&elm2.date));

    Ok((input, results))
}

fn parse_srs_forecast(input: &str) -> IResult<&str, Vec<SRSRBForecast>> {
    parse_srs_rb_forecast(input, "Solar Radiation Storm Forecast", 'S')
}

fn parse_rb_forecast(input: &str) -> IResult<&str, Vec<SRSRBForecast>> {
    parse_srs_rb_forecast(input, "Radio Blackout Forecast", 'R')
}

pub fn parse_sw_forecast(input: &str) -> Result<SWForecast, String> {
    let (input, kp_data) = parse_kp_forecast(input).finish().expect("Failed to parse text");
    let (input, srs_data) = parse_srs_forecast(input).finish().expect("Failed to parse text");
    let (_, rb_data) = parse_rb_forecast(input).finish().expect("Failed to parse text");
    Ok(SWForecast {
        kp: kp_data,
        srs: srs_data,
        rb: rb_data,
    })
}
