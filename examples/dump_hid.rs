fn main() {
    let api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            eprintln!("Failed to init HidApi: {}", e);
            return;
        }
    };
    for dev in api.device_list() {
        println!(
            "{:04x}:{:04x} interface {} path {:?}",
            dev.vendor_id(),
            dev.product_id(),
            dev.interface_number(),
            dev.path()
        );
    }
}
