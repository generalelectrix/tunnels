import org.msgpack.MessagePack;
import org.msgpack.packer.Packer;
import org.msgpack.unpacker.Unpacker;
import org.msgpack.annotation.Message;
import org.msgpack.template.Template;
import java.io.ByteArrayInputStream;
import java.util.List;
import org.zeromq.ZMQ;

// use alpha blending?  (expensive)
boolean useAlpha = false;
int frameNumber;

// Global deserialization
MessagePack msgpack = new MessagePack();

// Global 0mq frame receive socket
ZMQ.Context context = ZMQ.context(1);
ZMQ.Socket drawSocket = context.socket(ZMQ.SUB);

String serverAddress = "tcp://localhost:6000";

static int criticalSize;
static float thicknessScale = 0.5;
static float lineLengthScale = 6.0;
static int xCenter, yCenter, xSize, ySize;

void setup() {

    size(1280,720);
    criticalSize = 720;
    xSize = 1280;
    ySize = 720;

    /*
    size(1920,1080);
    criticalSize = 1080;
    xSize = 1920;
    ySize = 1080;
    */
    /*
    size(192,108);
    criticalSize = 108;
    xSize = 192;
    ySize = 108;
    */

    xCenter = xSize / 2;
    yCenter = ySize / 2;


    background(0); //black
    noSmooth();

    // turn off that annoying extra beam
    noCursor();

    ellipseMode(RADIUS);
    strokeCap(SQUARE);
    colorMode(HSB);
    //blendMode(ADD);

    frameRate(300.0);

    frameNumber = 0;

    // connect to the server and accept every message
    drawSocket.connect(serverAddress);

    byte[] filter = new byte[0];
    drawSocket.subscribe(filter);
}

void stop() {
    drawSocket.close();
    context.term();
}

/// Drain the incoming frame buffer and return the freshest frame.
List<Draw> getNewestFrame() throws IOException {
    // initial, blocking receive
    byte[] message;
    message = drawSocket.recv();
    // now drain the buffer
    while (true) {
        byte[] newestMessage = drawSocket.recv(ZMQ.DONTWAIT);
        if (newestMessage == null) {
            break;
        }
        else {
            message = newestMessage;
        }
    }

    // Unpack the msgpack draw commands
    ByteArrayInputStream byteStream = new ByteArrayInputStream(message);
    Unpacker unpacker = msgpack.createUnpacker(byteStream);
    // Python serializes as an array of two arrays
    unpacker.readArrayBegin();

    // Messages are packed as an array of type flags followed by an array of draw commands
    // These arrays should be the same length.
    // Draw flags are numeric for compactness.
    // 0 = arc
    // 1 = line

    int[] drawTypes = unpacker.read(int[].class);

    List<Draw> drawCalls = new ArrayList<Draw>();

    unpacker.readArrayBegin();

    for (int dt : drawTypes) {
        switch(dt) {
            case 0: // arc
                drawCalls.add(new DrawArc(unpacker.read(ParsedArc.class)));
                break;
            case 1: // line
                drawCalls.add(new DrawLine(unpacker.read(ParsedLine.class)));
                break;
        }
    }

    return drawCalls;
}

public static interface Draw {
    public abstract void draw();
}

class DrawArc implements Draw {
    ParsedArc params;

    DrawArc(ParsedArc ps) {
        params = ps;
    }

    void draw() {
        ParsedArc params = this.params;
        strokeWeight(params.thickness * criticalSize * thicknessScale);

        if (useAlpha) {
            stroke( color(params.hue, params.sat, params.val, params.level) );
        }
        else {
            color segColor = color(params.hue, params.sat, params.val);
            stroke( blendColor(segColor, color(0,0,params.level), MULTIPLY) );
        }

        pushMatrix();
        translate(params.x * xSize + xCenter, params.y * ySize + yCenter);
        rotate(params.rotAngle * TWO_PI);

        // draw pie wedge for this cell
        arc(0,
            0,
            params.radX * criticalSize,
            params.radY * criticalSize,
            params.start * TWO_PI,
            params.stop * TWO_PI);
        popMatrix();
    }
}

class DrawLine implements Draw {
    ParsedLine params;

    DrawLine(ParsedLine ps) {
        params = ps;
    }

    void draw() {
        ParsedLine params = this.params;
        strokeWeight(params.thickness * criticalSize * thicknessScale);

        if (useAlpha) {
            stroke( color(params.hue, params.sat, params.val, params.level) );
        }
        else {
            color segColor = color(params.hue, params.sat, params.val);
            stroke( blendColor(segColor, color(0,0,params.level), MULTIPLY) );
        }

        pushMatrix();
        translate(params.x * xSize + xCenter, params.y * ySize + yCenter);
        rotate(params.rotAngle * TWO_PI);

        // handle special cases to make beams appear nicely
        if (params.stop < params.start) {
            // TODO: lower size bound on drawing a line to avoid floating point
            // anomalies
            line((-0.5 + params.start) * params.length * criticalSize * lineLengthScale,
                 0,
                 0.5 * params.length * criticalSize * lineLengthScale,
                 0);
            line(-0.5 * params.length * criticalSize * lineLengthScale,
                 0,
                 (-0.5 + params.stop) * params.length * criticalSize * lineLengthScale,
                 0);
        }
        else {
            line((-0.5 + params.start) * params.length * criticalSize * lineLengthScale,
                 0,
                 (-0.5 + params.stop) * params.length * criticalSize * lineLengthScale,
                 0);
        }
        popMatrix();
    }
}

// MessagePack helper classes
@Message
static class ParsedArc {
    int level;
    float thickness;
    float hue;
    float sat;
    int val;
    float x;
    float y;
    float radX;
    float radY;
    float start;
    float stop;
    float rotAngle;
}

@Message
static class ParsedLine {
    int level;
    float thickness;
    float hue;
    float sat;
    int val;
    float x;
    float y;
    float length;
    float start;
    float stop;
    float rotAngle;
}


void draw() {

    background(0);
    noFill();

    //int startTime = millis();
    try {
        List<Draw> drawCalls = getNewestFrame();

        for (Draw toDraw: drawCalls) {
            toDraw.draw();
        }
    }
    catch (Exception e) {
        println("An exception ocurred: " + e.getMessage());
    }

    frameNumber++;
    //int endTime = millis();
    if (frameNumber % 30 == 0) {
        println(frameRate);
        //println(endTime - startTime);
    }
}