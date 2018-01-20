extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;

use async_dnssd::{register, RegisterFlag, Interface};
use futures::Future;
use tokio_core::reactor::Core;

fn main() {
    let core = Core::new().unwrap();

    let registration = register(
        RegisterFlag::Shared.into(),
        Interface::Any,
        None,
        "_tunnel._tcp",
        None,
        None,
        10000,
        "".as_bytes(),
        &core.handle()).unwrap().wait();

    println!("Running.");

    loop {
        ::std::thread::sleep_ms(10000);
    }

}
