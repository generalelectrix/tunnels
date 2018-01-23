extern crate zero_configure;

use zero_configure::Controller;


fn main() {
    let controller = Controller::new("testservice");

    let mut services = Vec::new();

    loop {
        let check = controller.list();
        if check != services {
            println!("Services: {:?}", check);
            services = check;
        }

        if !services.is_empty() {
            println!("Ping.");
            let result = controller.send(&services[0], &vec!(0)).unwrap();
            println!("Response: {:?}", result);
            break
        }
    }
}
