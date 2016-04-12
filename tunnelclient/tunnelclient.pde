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

Template parsedArcListTemplate = Templates.tList(msgpack.lookup(ParsedArc.class));

// Global 0mq frame receive socket
ZMQ.Context context = ZMQ.context(1);
ZMQ.Socket drawSocket = context.socket(ZMQ.SUB);

String serverAddress = "tcp://localhost:6000";

static int criticalSize;
static float thicknessScale = 0.5;
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

  List<ParsedArc> parsedArcs = unpacker.read(parsedArcListTemplate);
  List<Draw> drawArcs = new ArrayList<Draw>();
  for (ParsedArc pArc : parsedArcs) {
    drawArcs.add(new DrawArc(pArc));
  }
  return drawArcs;
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

// MessagePack helper class
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