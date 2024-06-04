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

// Common methods

fn parse_date(input: &str) -> IResult<&str, String> {
    let (input, month) = alpha1(input)?;
    let (input, _) = space1(input)?;
    let (input, day) = digit1(input)?;
    Ok((input, format!("{} {}", month, day)))
}

// parser that finds header and dates
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

// Kp forecast

fn parse_kp_val(input: &str) -> IResult<&str, f32> {
    let (input, kp_value) = float(input)?;
    let (input, _) = opt(space1)(input)?;
    let (input, _) = opt(delimited(tag("("), alphanumeric1, tag(")")))(input)?;
    Ok((input, kp_value))
}

fn parse_hours_interval(input: &str) -> IResult<&str, (u8, u8)> {
    let (input, start) = digit1(input)?;
    let (input, _) = tag("-")(input)?;
    let (input, end) = digit1(input)?;
    let (input, _) = tag("UT")(input)?;
    Ok((input, (u8::from_str(start).unwrap(), u8::from_str(end).unwrap())))
}

// parser for rows with interval and Kp value
fn parse_kp_row(input: &str) -> IResult<&str, (u8, u8, Vec<f32>)> {
    let (input, (time_range_start, time_range_end)) = parse_hours_interval(input)?;
    let (input, kps) = many1(preceded(space0, parse_kp_val))(input)?;
    let (input, _) = opt(multispace1)(input)?;
    let (input, _) = opt(line_ending)(input)?;
    Ok((input, (time_range_start, time_range_end, kps)))
}

fn parse_kp_forecast(input: &str) -> IResult<&str, Vec<KPForecast>> {
    let (input, dates) = parse_header(input, "NOAA Kp index breakdown")?;
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

// Solar and Radio Blackout stroms forecast

fn parse_prcnt_val(input: &str) -> IResult<&str, u8> {
    let (input, value) = digit1(input)?;
    let (input, _) = tag("%")(input)?;
    Ok((input, u8::from_str(value).unwrap()))
}

// Parser that returns max and min storm grades.
// If it finds "or greater" phrase then max storm grade equals min grade + 1
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

// parser for rows with min/max storm grades and value of storm probability
fn parse_srs_rb_row(input: &str, storm_type: char) -> IResult<&str, (u8, u8, Vec<u8>)> {
    let (input, (s_min, s_max)) = parse_solar_rb_storms(input, storm_type)?;
    let (input, values) = many1(preceded(space0, parse_prcnt_val))(input)?;
    let (input, _) = opt(multispace1)(input)?;
    let (input, _) = opt(line_ending)(input)?;
    Ok((input, (s_min, s_max, values)))
}

// common parser for SRS and RB forecasts
fn parse_srs_rb_forecast<'a>(input: &'a str, header_phrase: &str, storm_type: char) -> IResult<&'a str, Vec<SRSRBForecast>> {
    let (input, dates) = parse_header(input, header_phrase)?;
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

// Public interface

// Parser for 3 day space weather forecast from NOAA text data.
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

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    const SW_FORECAST_DATA1: &str = "
:Product: 3-Day Forecast
:Issued: 2024 May 01 0030 UTC
# Prepared by the U.S. Dept. of Commerce, NOAA, Space Weather Prediction Center
#
A. NOAA Geomagnetic Activity Observation and Forecast

The greatest observed 3 hr Kp over the past 24 hours was 4 (below NOAA
Scale levels).
The greatest expected 3 hr Kp for May 01-May 03 2024 is 4.67 (NOAA Scale
G1).

NOAA Kp index breakdown May 01-May 03 2024

             May 01       May 02       May 03
00-03UT       4.67 (G1)    3.67         3.67     
03-06UT       4.00         4.00         3.33     
06-09UT       3.00         3.67         3.00     
09-12UT       2.33         3.33         3.33     
12-15UT       2.67         6.00 (G2)    3.00     
15-18UT       2.33         2.67         3.33     
18-21UT       3.00         3.67         3.33     
21-00UT       3.33         3.67         8.67 (G4)

Rationale: G1 (Minor) geomagnetic storming is expected during the early
hours of 01 May due to transient influences.

B. NOAA Solar Radiation Activity Observation and Forecast

Solar radiation, as observed by NOAA GOES-18 over the past 24 hours, was
below S-scale storm level thresholds.

Solar Radiation Storm Forecast for May 01-May 03 2024

              May 01  May 02  May 03
S1 or greater    5%      5%      5%

Rationale: No S1 (Minor) or greater solar radiation storms are expected.
No significant active region activity favorable for radiation storm
production is forecast.

C. NOAA Radio Blackout Activity and Forecast

Radio blackouts reaching the R2 levels were observed over the past 24
hours. The largest was at Apr 30 2024 2346 UTC.

Radio Blackout Forecast for May 01-May 03 2024

              May 01        May 02        May 03
R1-R2           55%           45%           35%
R3 or greater   10%           10%            5%

Rationale: R1-2 (Minor-Moderate) radio blackouts due to M-class flare
activity primarily from AR 3654 are likely on 01 May.
";

    #[test]
    fn test_parse_kp_forecast() {
        #[rustfmt::skip]
        let kp_forecast1: Vec<KPForecast> = vec![
            KPForecast { date: "May 01 2024".to_string(), hour: 3, value: 4.67 },
            KPForecast { date: "May 01 2024".to_string(), hour: 6, value: 4.0 },
            KPForecast { date: "May 01 2024".to_string(), hour: 9, value: 3.0 },
            KPForecast { date: "May 01 2024".to_string(), hour: 12, value: 2.33 },
            KPForecast { date: "May 01 2024".to_string(), hour: 15, value: 2.67 },
            KPForecast { date: "May 01 2024".to_string(), hour: 18, value: 2.33 },
            KPForecast { date: "May 01 2024".to_string(), hour: 21, value: 3.00 },
            KPForecast { date: "May 01 2024".to_string(), hour: 0, value: 3.33 },

            KPForecast { date: "May 02 2024".to_string(), hour: 3, value: 3.67 },
            KPForecast { date: "May 02 2024".to_string(), hour: 6, value: 4.0 },
            KPForecast { date: "May 02 2024".to_string(), hour: 9, value: 3.67 },
            KPForecast { date: "May 02 2024".to_string(), hour: 12, value: 3.33 },
            KPForecast { date: "May 02 2024".to_string(), hour: 15, value: 6.0 },
            KPForecast { date: "May 02 2024".to_string(), hour: 18, value: 2.67 },
            KPForecast { date: "May 02 2024".to_string(), hour: 21, value: 3.67 },
            KPForecast { date: "May 02 2024".to_string(), hour: 0, value: 3.67 },

            KPForecast { date: "May 03 2024".to_string(), hour: 3, value: 3.67 },
            KPForecast { date: "May 03 2024".to_string(), hour: 6, value: 3.33 },
            KPForecast { date: "May 03 2024".to_string(), hour: 9, value: 3.0 },
            KPForecast { date: "May 03 2024".to_string(), hour: 12, value: 3.33 },
            KPForecast { date: "May 03 2024".to_string(), hour: 15, value: 3.0 },
            KPForecast { date: "May 03 2024".to_string(), hour: 18, value: 3.33 },
            KPForecast { date: "May 03 2024".to_string(), hour: 21, value: 3.33 },
            KPForecast { date: "May 03 2024".to_string(), hour: 0, value: 8.67 },
        ];
        let (_, kp_data) = parse_kp_forecast(SW_FORECAST_DATA1).finish().unwrap();
        for i in 0..kp_forecast1.len() {
            assert_eq!(kp_forecast1[i].date, kp_data[i].date);
            assert_eq!(kp_forecast1[i].hour, kp_data[i].hour);
            assert_eq!(kp_forecast1[i].value, kp_data[i].value);
        }
    }

    #[test]
    fn test_parse_srs_forecast() {
        #[rustfmt::skip]
        let srs_forecast1: Vec<SRSRBForecast> = vec![
            SRSRBForecast { date: "May 01 2024".to_string(), s1: 5, s2: 5, s3: 0, s4: 0, s5: 0, },
            SRSRBForecast { date: "May 02 2024".to_string(), s1: 5, s2: 5, s3: 0, s4: 0, s5: 0, },
            SRSRBForecast { date: "May 03 2024".to_string(), s1: 5, s2: 5, s3: 0, s4: 0, s5: 0, },
        ];
        let (_, data) = parse_srs_forecast(SW_FORECAST_DATA1).finish().unwrap();
        for i in 0..srs_forecast1.len() {
            assert_eq!(srs_forecast1[i].date, data[i].date);
            assert_eq!(srs_forecast1[i].s1, data[i].s1);
            assert_eq!(srs_forecast1[i].s2, data[i].s2);
            assert_eq!(srs_forecast1[i].s3, data[i].s3);
            assert_eq!(srs_forecast1[i].s4, data[i].s4);
            assert_eq!(srs_forecast1[i].s5, data[i].s5);
        }
    }

    #[test]
    fn test_parse_rb_forecast() {
        #[rustfmt::skip]
        let rb_forecast1: Vec<SRSRBForecast> = vec![
            SRSRBForecast { date: "May 01 2024".to_string(), s1: 55, s2: 55, s3: 10, s4: 10, s5: 0, },
            SRSRBForecast { date: "May 02 2024".to_string(), s1: 45, s2: 45, s3: 10, s4: 10, s5: 0, },
            SRSRBForecast { date: "May 03 2024".to_string(), s1: 35, s2: 35, s3: 5, s4: 5, s5: 0, },
        ];
        let (_, data) = parse_rb_forecast(SW_FORECAST_DATA1).finish().unwrap();
        for i in 0..rb_forecast1.len() {
            assert_eq!(rb_forecast1[i].date, data[i].date);
            assert_eq!(rb_forecast1[i].s1, data[i].s1);
            assert_eq!(rb_forecast1[i].s2, data[i].s2);
            assert_eq!(rb_forecast1[i].s3, data[i].s3);
            assert_eq!(rb_forecast1[i].s4, data[i].s4);
            assert_eq!(rb_forecast1[i].s5, data[i].s5);
        }
    }
}
