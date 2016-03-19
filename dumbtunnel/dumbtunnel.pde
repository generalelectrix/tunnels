import org.msgpack.MessagePack;
import org.msgpack.packer.Packer;
import org.msgpack.unpacker.Unpacker;
import org.msgpack.annotation.Message;
import org.msgpack.template.Template;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.ByteArrayInputStream;
import java.util.*;
import java.nio.file.*;
import org.zeromq.ZMQ;

int x_size = 1280;
int y_size = 720;

MessagePack msgpack = new MessagePack();

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

ZMQ.Context context = ZMQ.context(1);

//  Socket to talk to server
ZMQ.Socket drawSocket = context.socket(ZMQ.SUB);

void setup() {
  
  //size(1280,720, FX2D);
  size(1280,720);

  background(0); //black
  noSmooth();
  
  ellipseMode(RADIUS);
  
  strokeCap(SQUARE);
  
  frameRate(300.0);
  
  colorMode(HSB);
  
  //blendMode(ADD);
  
  frameNumber = 0;
  
  drawSocket.connect("tcp://localhost:6000");

  byte[] filter = new byte[0];
  drawSocket.subscribe(filter);
}

void stop() {
  drawSocket.close();
  context.term();
}
String testPattern = "/Users/Chris/src/pytunnel/testpattern.csv";
String layer0 = "/Users/Chris/src/pytunnel/layer0.csv";
String drawFile = layer0;

Path drawFilePath = Paths.get(drawFile);

Table drawTable;

int frameNumber;

boolean useAlpha = false;

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
  
  ByteArrayInputStream byteStream = new ByteArrayInputStream(message);
  Unpacker unpacker = msgpack.createUnpacker(byteStream);
    
  return unpacker.read(arcListTemplate);
}

// method called whenever processing draws a frame, basically the event loop
void draw() {

  
  /*
  drawTable = loadTable(drawFile);
  
  for (TableRow row : drawTable.rows()) {
    int level = row.getInt(0);
    float strokeWeight_ = row.getFloat(1);
    float hue_ = row.getFloat(2);
    float sat = row.getFloat(3);
    int val = row.getInt(4);
    int x = row.getInt(5);
    int y = row.getInt(6);
    int radX = row.getInt(7);
    int radY = row.getInt(8);
    float start = row.getFloat(9);
    float stop = row.getFloat(10);
  */
  
  background(0);
  
  noFill();
  
  //int startTime = millis();
  try {
    //FileInputStream inputFile = new FileInputStream(drawFile);
    //byte[] drawBytes = Files.readAllBytes(drawFilePath);
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