use super::*;

#[test]
fn baud_rate_suggestions_are_numerically_ordered_unique_and_allow_custom_values() {
    let mut options = SerialOptions::baud_rates();
    let rates: Vec<_> = (0..options.items_count(0))
        .map(|index| {
            options
                .item(IndexPath::new(index))
                .unwrap()
                .parse::<u32>()
                .unwrap()
        })
        .collect();
    assert!(rates.windows(2).all(|pair| pair[0] < pair[1]));
    for rate in [9600, 19200, 38400, 57600, 115200, 230400, 921600] {
        assert!(rates.contains(&rate));
    }
    assert!(!rates.contains(&0));
    options.retain_value("123457");
    assert!(options.position(&"123457".to_owned()).is_some());
}

#[test]
fn port_names_sort_deduplicate_and_prefer_existing_macos_callout_aliases() {
    let names = [
        "/dev/tty.usbserial-2",
        "/dev/cu.usbserial-2",
        "/dev/tty.unpaired",
        "/dev/cu.usbserial-1",
        "/dev/cu.usbserial-2",
    ]
    .map(PathBuf::from)
    .to_vec();
    assert_eq!(
        port_names(names.clone(), true),
        [
            "/dev/cu.usbserial-1",
            "/dev/cu.usbserial-2",
            "/dev/tty.unpaired",
        ]
    );
    assert_eq!(
        port_names(names, false),
        [
            "/dev/cu.usbserial-1",
            "/dev/cu.usbserial-2",
            "/dev/tty.unpaired",
            "/dev/tty.usbserial-2",
        ]
    );
    assert_eq!(
        port_names(["COM7", "COM3", "COM7"].map(PathBuf::from).to_vec(), false),
        ["COM3", "COM7"]
    );
    assert!(port_names(Vec::new(), true).is_empty());
}

#[test]
fn port_options_keep_manual_and_disconnected_values_without_duplicates() {
    let mut options = SerialOptions::new(vec!["COM7".into(), "COM3".into(), "COM7".into()]);
    assert_eq!(options.items_count(0), 2);
    assert_eq!(options.items_count(1), 0);
    assert_eq!(
        options.item(IndexPath::new(0)).map(String::as_str),
        Some("COM3")
    );
    assert!(options.item(IndexPath::new(9)).is_none());
    assert_eq!(
        options.position(&"COM7".to_owned()),
        Some(IndexPath::new(1))
    );
    assert!(options.position(&"COM19".to_owned()).is_none());
    options.retain_value("COM19");
    options.retain_value("COM19");
    assert_eq!(options.items_count(0), 3);
    assert!(options.position(&"COM19".to_owned()).is_some());
    let mut refreshed = SerialOptions::new(Vec::new());
    refreshed.retain_value("COM19");
    assert_eq!(refreshed.items_count(0), 1);
    assert_eq!(refreshed.item(IndexPath::new(0)).unwrap(), "COM19");
}
