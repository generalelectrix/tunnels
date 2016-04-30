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

/// Unpack a yet-to-be-determined shape or shape collection.
List<Draw> unpackEntity(Unpacker unpacker) throws IOException {
    // unpack the opening brace
    unpacker.readArrayBegin();

    // read the type flag
    int type = unpacker.readInt();

    List<Draw> unpacked;

    if (type == 0) {
        unpacked = unpackShapeCollection(unpacker);
    }
    else {
        unpacked = unpackShape(unpacker, type);
    }

    // unpack the closing brace
    unpacker.readArrayEnd();

    return unpacked;
}

/// Unpack the internals of a serialized shape.
List<Draw> unpackShape(Unpacker unpacker, int shapeType) throws IOException {

    List<Draw> toDraw = new ArrayList<Draw>();

    // switch on shape type flag and parse the resuling array
    switch(shapeType) {
        case 1: // arcs
            List<ParsedArc> arcs = unpacker.read(parsedArcListTmpl);
            for (ParsedArc arc : arcs) {
                toDraw.add(new DrawArc(arc));
            }
            break;
        case 2: // lines
            List<ParsedLine> lines = unpacker.read(parsedLineListTmpl);
            for (ParsedLine line : lines) {
                toDraw.add(new DrawLine(line));
            }
            break;
    }

    return toDraw;
}

/// Unpack a serialzed ShapeCollection.
List<Draw> unpackShapeCollection(Unpacker unpacker) throws IOException {
    // unpack the number of entities in this collection
    int nEntities = unpacker.readInt();

    // unpack the opening brace
    unpacker.readArrayBegin();

    List<Draw> toDraw = new ArrayList<Draw>();

    // loop over the expected number of entities and unpack them
    for (int i = 0; i < nEntities; i++) {
        toDraw.addAll(unpackEntity(unpacker));
    }

    // unpack the closing brace
    unpacker.readArrayEnd();

    return toDraw;
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

    // read the opening brace
    unpacker.readArrayBegin();

    // read the message header, consisting of the frame number and frame time
    int frameNumber = unpacker.readInt();
    long frameTime = unpacker.readLong();

    // read the list of draw commands
    List<Draw> toDraw = unpackEntity(unpacker);

    // read the closing brace, for completeness
    unpacker.readArrayEnd();

    return toDraw;
}

/// Encapsulate a stack o' draw calls and handle interpolation.
class TunnelShape implements Draw {
    List<DrawArc> drawArcs;

    TunnelShape(List<ParsedArc> drawParams) {
        this.drawArcs = new ArrayList<DrawArc>();
        for (ParsedArc params : drawParams) {
            this.drawArcs.add(new DrawArc(params));
        }
    }

    /// Interpolate this TunnelShape with another
    Draw interpolateWith(TunnelShape interpWith, double alpha) {
        // if the two have differing numbers of draw calls, use whichever is
        // closer to alpha
        if (this.drawArcs.size() != interpWith.drawArcs.size()){
            if (alpha < 0.5) {
                return this;
            }
            else {
                return interpWith;
            }
        }
        // otherwise, interpolate!
        return this;
    }

    void draw() {
        for (DrawArc da : this.drawArcs) {
            da.draw();
        }
    }
}

public static interface Draw {
    public abstract void draw();
}

int interp(int x1, int x2, double alpha) {
    return int(x1 * (1.0 - alpha) + x2 * (alpha))
}

float interp(float x1, float x2, double alpha) {
    return float(x1 * (1.0 - alpha) + x2 * (alpha))
}

/// Interpolate angles correctly.
float interpRadial(float r1, float r2, double alpha) {

}

class DrawArc implements Draw {
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

    DrawArc(ParsedArc ps) {
        level = ps.level;
        thickness = ps.thickness;
        hue = ps.hue;
        sat = ps.sat;
        val = ps.val;
        x = ps.x;
        y = ps.y;
        radX = ps.radX;
        radY = ps.radY;
        start = ps.start;
        stop = ps.stop;
        rotAngle = ps.rotAngle;
    }

    /// Interpolate this DrawArc with another
    DrawArc interpolateWith(DrawArc other, double alpha) {
        return DrawArc(
            interp(this.level, other.level, alpha),
            interp(this.thickness, other.thickness, alpha),

            )
    }

    void draw() {
        strokeWeight(this.thickness * criticalSize * thicknessScale);

        if (useAlpha) {
            stroke( color(this.hue, this.sat, this.val, this.level) );
        }
        else {
            color segColor = color(this.hue, this.sat, this.val);
            stroke( blendColor(segColor, color(0,0,this.level), MULTIPLY) );
        }

        pushMatrix();
        translate(this.x * xSize + xCenter, this.y * ySize + yCenter);
        rotate(this.rotAngle * TWO_PI);

        float start = this.start;
        float stop = this.stop;

        // draw pie wedge for this cell
        arc(0,
            0,
            this.radX * criticalSize,
            this.radY * criticalSize,
            this.start * TWO_PI,
            this.stop * TWO_PI);
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

        float start = params.start;
        float stop = params.stop % 1.0;

        // handle special cases to make beams appear nicely
        if (stop < start) {
            // TODO: lower size bound on drawing a line to avoid floating point
            // anomalies
            line((-0.5 + start) * params.length * criticalSize * lineLengthScale,
                 0,
                 0.5 * params.length * criticalSize * lineLengthScale,
                 0);
            line(-0.5 * params.length * criticalSize * lineLengthScale,
                 0,
                 (-0.5 + stop) * params.length * criticalSize * lineLengthScale,
                 0);
        }
        else {
            line((-0.5 + start) * params.length * criticalSize * lineLengthScale,
                 0,
                 (-0.5 + stop) * params.length * criticalSize * lineLengthScale,
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

Template parsedArcListTmpl = Templates.tList(msgpack.lookup(ParsedArc.class));
Template parsedLineListTmpl = Templates.tList(msgpack.lookup(ParsedLine.class));

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