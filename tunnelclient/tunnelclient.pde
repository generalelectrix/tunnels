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

// MessagePack helper class
@Message
public static class DrawArc {
  public int level;
  public float thickness;
  public float hue;
  public float sat;
  public int val;
  public float x;
  public float y;
  public float radX;
  public float radY;
  public float start;
  public float stop;
}

Template arcListTemplate = Templates.tList(msgpack.lookup(DrawArc.class));

// Global 0mq frame receive socket
ZMQ.Context context = ZMQ.context(1);
ZMQ.Socket drawSocket = context.socket(ZMQ.SUB);

String serverAddress = "tcp://localhost:6000";

/// Drain the incoming frame buffer and return the freshest frame.
List<DrawArc> getNewestFrame() throws IOException {
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

  return unpacker.read(arcListTemplate);
}

int criticalSize;
float thicknessScale = 0.5;
int xCenter, yCenter, xSize, ySize;

void setup() {
  /*
  size(1280,720);
  criticalSize = 720;
  xSize = 1280;
  ySize = 720;
  */

  size(1920,1080);
  criticalSize = 1080;
  xSize = 1920;
  ySize = 1080;

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

void draw() {

  background(0);
  noFill();

  //int startTime = millis();
  try {
    List<DrawArc> arcs = getNewestFrame();

      for (DrawArc toDraw: arcs) {

        strokeWeight(toDraw.thickness * criticalSize * thicknessScale);

        if (useAlpha) {
          stroke( color(toDraw.hue, toDraw.sat, toDraw.val, toDraw.level) );
        }
        else {
          color segColor = color(toDraw.hue, toDraw.sat, toDraw.val);
          stroke( blendColor(segColor, color(0,0,toDraw.level), MULTIPLY) );
        }

        // draw pie wedge for this cell
        arc(toDraw.x * xSize + xCenter,
            toDraw.y * ySize + yCenter,
            toDraw.radX * criticalSize,
            toDraw.radY * criticalSize,
            toDraw.start * TWO_PI,
            toDraw.stop * TWO_PI);
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