extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;

use async_dnssd::{browse, BrowsedFlag, Interface};
use futures::Stream;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();

    let handle = core.handle();

    let browse_result = browse(
                                Interface::Any,
                                "_tunnel._tcp",
                                None,
                                &handle)
        .unwrap()
        .filter_map(|item| {
            println!("{:?}", item);
            // check if this service was added
            if item.flags & BrowsedFlag::Add {
                Some(item)
            } else {
                None
            }
        })
        .and_then(|item| item.resolve(&handle))
        .flatten()
        .for_each(|item| {
            println!("{:?}", item);
            Ok(())
        });

    core.run(browse_result).unwrap();
}
