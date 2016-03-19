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
  public float strokeWeight;
  public float hue;
  public float sat;
  public int val;
  public int x;
  public int y;
  public int radX;
  public int radY;
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

void setup() {
  
  //size(1280,720, FX2D);
  size(1280,720);

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
        
        strokeWeight(toDraw.strokeWeight);
        
        if (useAlpha) {
          stroke( color(toDraw.hue, toDraw.sat, toDraw.val, toDraw.level) );  
        }
        else {
          color segColor = color(toDraw.hue, toDraw.sat, toDraw.val);
          stroke( blendColor(segColor, color(0,0,toDraw.level), MULTIPLY) );
        }
      
        // draw pie wedge for this cell
        arc(toDraw.x, toDraw.y, toDraw.radX, toDraw.radY, toDraw.start, toDraw.stop);
        
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