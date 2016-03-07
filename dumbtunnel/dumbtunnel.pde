int x_size = 1280;
int y_size = 720;

void setup() {
  
  size(1280,720, FX2D);

  background(0); //black
  noSmooth();
  
  ellipseMode(RADIUS);
  
  strokeCap(SQUARE);
  
  frameRate(60);
  
  colorMode(HSB);
  
  blendMode(ADD);
  
  frameNumber = 0;
}
String testPattern = "/Users/fionakirkpatrick/src/pytunnel/testpattern.csv";
String layer0 = "/Users/fionakirkpatrick/src/pytunnel/layer0.csv";
String drawFile = layer0;

Table drawTable;

int frameNumber;

boolean useAlpha = false;

// method called whenever processing draws a frame, basically the event loop
void draw() {
  
  background(0);
  
  noFill();
  
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
 
  }
  frameNumber++;
  if (frameNumber % 30 == 0) {
    println(frameRate);
  }
}