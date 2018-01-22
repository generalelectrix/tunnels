extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;
extern crate zmq;

use async_dnssd::{register, RegisterFlag, Interface, Registration};
use futures::Future;
use tokio_core::reactor::{Core, Handle};

use zmq::{Context, Socket};

use std::net::ToSocketAddrs;
use std::net::IpAddr;
use std::error::Error;

/// Synchronously register a service to DNS-SD.
fn register_service(reg_type: &str, port: u16, handle: &Handle) {

}

/// Advertise a service over DNS-SD, using a 0mq REQ/REP socket as the subsequent transport.
pub struct Service {
    registration: Registration,
    socket: Socket,
}

impl Service {
    pub fn start(name: &str, port: u16, ctx: &mut Context) -> Result<Self, Box<Error>> {

        // Open the 0mq socket we'll use to service requests.
        let socket = ctx.socket(zmq::REP)?;
        let addr = format!("tcp://*:{}", port);
        socket.connect(&addr)?;

        // Create a tokio core just to run this one future.
        let core = Core::new()?;

        let reg_type = format!("_{}._tcp", name);

        // Start advertising this service over DNS-SD.
        let (registration, _) = register(
            RegisterFlag::Shared.into(),
            Interface::Any,
            None,
            &reg_type,
            None,
            None,
            port,
            "".as_bytes(),
            &core.handle())?
            .wait()?;

        Ok(Service {
            registration,
            socket,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
