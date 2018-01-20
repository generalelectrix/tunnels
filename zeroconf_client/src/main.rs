extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;

use async_dnssd::{browse, Interface};
use futures::Stream;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();

    let browse_result = browse(
                                Interface::Any,
                                "_tunnel._tcp",
                                None,
                                &core.handle())
        .unwrap()
        .for_each(|item| {
            println!("{:?}", item);
            Ok(())
        });

    core.run(browse_result).unwrap();
}
