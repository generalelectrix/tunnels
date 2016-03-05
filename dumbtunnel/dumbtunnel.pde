int x_size = 1280;
int y_size = 720;

void setup() {
  
  size(x_size,y_size);

  background(0); //black
  //smooth(); // anti-aliasing is SLOW
  
  ellipseMode(RADIUS);
  
  strokeCap(SQUARE);
  
  frameRate(20);
  
  colorMode(HSB);
  
  frameNumber = 0;
}
String testPattern = "/Users/Chris/src/pytunnel/testpattern.csv";
String layer0 = "/Users/Chris/src/pytunnel/layer0.csv";
String drawFile = layer0;

Table drawTable;

int frameNumber;

// method called whenever processing draws a frame, basically the event loop
void draw() {
  
  background(0);
  
  noFill();
  
  drawTable = loadTable(drawFile);
  
  for (TableRow row : drawTable.rows()) {
    int level = row.getInt(0);
    boolean stroke_ = boolean(row.getInt(1));
    float strokeWeight_ = row.getFloat(2);
    float hue_ = row.getFloat(3);
    float sat = row.getFloat(4);
    int val = row.getInt(5);
    int x = row.getInt(6);
    int y = row.getInt(7);
    int radX = row.getInt(8);
    int radY = row.getInt(9);
    float start = row.getFloat(10);
    float stop = row.getFloat(11);
    
    color segColor = color(hue_, sat, val);

    // only draw something if the segment color isn't black.
    if (stroke_) {
      strokeWeight(strokeWeight_);
      stroke( blendColor(segColor, color(0,0,level), MULTIPLY) );
    
      // draw pie wedge for this cell
      arc(x, y, radX, radY, start, stop);
    }
  }
  frameNumber++;
  println(frameRate);
}

