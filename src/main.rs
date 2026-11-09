use kittx::obd::Pid;
use kittx::{Client, TransportConfig};
use std::time::Duration;

fn main() {
    let mut client = Client::new(TransportConfig::tcp("127.0.0.1", 36000));
    client.connect(Some(Duration::from_millis(1000)), |result| result.unwrap());
    client.read_voltage(Some(Duration::from_millis(1000)), |result| {
        println!("{result:?}")
    });
    client.read_pid(Pid::EngineRpm, Some(Duration::from_secs(10)), |result| {
        for measurement in result.unwrap() {
            println!("{:?}: {} rpm", measurement.sender, measurement.value);
        }
    });
    client.disconnect(None, |result| result.unwrap());
}
