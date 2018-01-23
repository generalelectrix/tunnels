extern crate zero_configure;

use zero_configure::run_service;


fn main() {
    run_service("testservice", 11000, |_| {
        vec!(1, 2, 3, 4, 5)
    }).unwrap();
}
