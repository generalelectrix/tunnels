@0xad68083eb134d297;

struct Arc {
    level @0 :UInt32;
    strokeWeight @1 :Float32;
    hue @2 :Float32;
    sat @3 :Float32;
    val @4 :UInt32;
    x @5 :Int32;
    y @6 :Int32;
    radX @7 :UInt32;
    radY @8 :UInt32;
    start @9 :Float32;
    stop @10: Float32;
}

struct DrawCommands {
    arcs @0 :List(Arc);
}