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

void setup() {
  
  size(1280,720, FX2D);
  //size(1280,720);

  background(0); //black
  noSmooth();
  
  ellipseMode(RADIUS);
  
  strokeCap(SQUARE);
  
  frameRate(10.0);
  
  colorMode(HSB);
  
  //blendMode(ADD);
  
  frameNumber = 0;

}
String testPattern = "/Users/Chris/src/pytunnel/testpattern.csv";
String layer0 = "/Users/Chris/src/pytunnel/layer0.csv";
String drawFile = layer0;

Path drawFilePath = Paths.get(drawFile);

Table drawTable;

int frameNumber;

boolean useAlpha = false;

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
    byte[] drawBytes = Files.readAllBytes(drawFilePath);
    ByteArrayInputStream byteStream = new ByteArrayInputStream(drawBytes);
    Unpacker unpacker = msgpack.createUnpacker(byteStream);
    
    List<DrawArc> arcs = unpacker.read(arcListTemplate);
    for (DrawArc arc: arcs) {
      
      strokeWeight(arc.strokeWeight);
      
      if (useAlpha) {
        stroke( color(arc.hue, arc.sat, arc.val, arc.level) );  
      }
      else {
        color segColor = color(arc.hue, arc.sat, arc.val);
        stroke( blendColor(segColor, color(0,0,arc.level), MULTIPLY) );
      }
    
      // draw pie wedge for this cell
      arc(arc.x, arc.y, arc.radX, arc.radY, arc.start, arc.stop);
      
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