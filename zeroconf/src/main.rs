extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;

use async_dnssd::{register, RegisterFlag, Interface};
use futures::Future;
use tokio_core::reactor::Core;

fn main() {
    let core = Core::new().unwrap();

    let registration = register(
        RegisterFlag::Unique.into(),
        Interface::Any,
        Some("tunnel_server"),
        "_http._tcp",
        None,
        None,
        10000,
        "".as_bytes(),
        &core.handle()).unwrap().wait();

    loop {
        ::std::thread::sleep_ms(10000);
    }

}
