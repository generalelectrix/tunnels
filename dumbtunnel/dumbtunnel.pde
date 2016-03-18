import org.msgpack.MessagePack;
import org.msgpack.unpacker.Unpacker;
import java.io.FileInputStream;

int x_size = 1280;
int y_size = 720;

MessagePack msgpack = new MessagePack();

void setup() {
  
  size(1280,720, FX2D);
  //size(1280,720);

  background(0); //black
  noSmooth();
  
  ellipseMode(RADIUS);
  
  strokeCap(SQUARE);
  
  frameRate(30);
  
  colorMode(HSB);
  
  //blendMode(ADD);
  
  frameNumber = 0;
}
String testPattern = "/Users/Chris/src/pytunnel/testpattern.csv";
String layer0 = "/Users/Chris/src/pytunnel/layer0.csv";
String drawFile = layer0;

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
  
  int startTime = millis();
  try {
    FileInputStream inputFile = new FileInputStream(drawFile);
    Unpacker unpacker = msgpack.createUnpacker(inputFile);
    int nCalls = unpacker.readArrayBegin();
    
    for (int i=0; i<nCalls; i++) {
      unpacker.readArrayBegin();
      int level = unpacker.readInt();
      float strokeWeight_ = unpacker.readFloat();
      float hue_ = unpacker.readFloat();
      float sat = unpacker.readFloat();
      int val = unpacker.readInt();
      int x = unpacker.readInt();
      int y = unpacker.readInt();
      int radX = unpacker.readInt();
      int radY = unpacker.readInt();
      float start = unpacker.readFloat();
      float stop = unpacker.readFloat();
      unpacker.readArrayEnd();
      /*
      strokeWeight(strokeWeight_);
      
      if (useAlpha) {
        stroke( color(hue_, sat, val, level) );  
      }
      else {
        color segColor = color(hue_, sat, val);
        stroke( blendColor(segColor, color(0,0,level), MULTIPLY) );
      }
    
      // draw pie wedge for this cell
      arc(x, y, radX, radY, start, stop);
      */
    }
    unpacker.readArrayEnd();
    inputFile.close();
  }
  catch (Exception e) {
    println("An exception ocurred: " + e.getMessage());
  }
  frameNumber++;
  int endTime = millis();
  if (frameNumber % 30 == 0) {
    println(frameRate);
    println(endTime - startTime);
  }
}