## Mulit Weather provider

### Description

Multi weather provider is backend that fetches weather data from internet and send it to MQTT broker. Now it mostly space weather provider, but it was conceived as multi weather provider: both usual and space.

Provides weather resources:
- NOAA Kp index instant value https://services.swpc.noaa.gov/json/planetary_k_index_1m.json
- NOAA Kp index history data https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json
- NOAA Solar Radiation flux data https://services.swpc.noaa.gov/json/goes/primary/integral-protons-plot-6-hour.json
- NOAA space weather forecast: Geomagnetic Storms, Solar Radiation Storms https://services.swpc.noaa.gov/text/3-day-forecast.txt

Space weather forecast parses from text weather report from NOAA.

You can find example of how to work with weather provider in 'Slint meteo GUI' https://github.com/TEPOTPOH/slint-meteo-gui .

### Configuration

- Use environment variable `MQTT_BROKER_HOST` to set MQTT broker host. By default it is `localhost`.
- Use environment variable `MQTT_BROKER_PORT` to set MQTT broker port. By default it is `1883`.

### Building and running

`cargo run`

### TODO:
- [x] Write tests on parser for space weather report
- [ ] Add getting weather forecast from Windy or Yandex Weather
- [ ] Limit max time for loading and sending resource
- [ ] Move to config some constants from conveters.rs
- [ ] Configure user name and password for MQTT broker