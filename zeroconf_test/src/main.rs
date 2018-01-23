extern crate zero_configure;

use zero_configure::{run_service, ServiceDefinition};


fn main() {
    let mut sd = ServiceDefinition::new("testservice", 11000);
    sd.localhost_only = true;
    run_service(sd, |_| {
        vec!(1, 2, 3, 4, 5)
    }).unwrap();
}
