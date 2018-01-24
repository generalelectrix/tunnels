/// Enable remote control of a tunnel render slave over the network.
/// Advertises this slave for control over DNS-SD, handling requests on a 0mq socket.
/// Very basic control; every message received is a full configuration struct, and the receipt of
/// a message completely tears down an existing show and brings up a new one using the new
/// parameters.