extern crate zero_configure;

use zero_configure::Controller;


fn main() {
    let controller = Controller::new("test_service");

    let mut services = Vec::new();

    loop {
        let check = controller.list();
        if check != services {
            println!("Services: {}", check);
            services = check;
        }
    }
}
