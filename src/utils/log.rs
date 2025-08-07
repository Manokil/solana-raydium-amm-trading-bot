use chrono::Local;

pub fn log_stamp(arg: &str) {
    let now = Local::now();

    let date = now.format("%Y-%m-%d").to_string();
    let hour = now.format("%H").to_string();
    let minute = now.format("%M").to_string();
    let second = now.format("%S").to_string();
    let millis = format!("{:03}", now.timestamp_subsec_millis());      // 123
    let nanos = format!("{:03}", now.timestamp_subsec_nanos());        // 000000123 if only 123 ns

    println!("{} => {} {} {} {} {} {}", arg, date, hour, minute, second, millis, nanos);
}