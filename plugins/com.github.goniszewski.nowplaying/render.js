// Draws one 72x72 key for the Now Playing plugin with AppKit and writes it as a PNG.
// Run as: osascript -l JavaScript render.js <out.png> <playing|paused|stopped> [<artwork file>]
// - playing: the cover, or three bars when there is none.
// - paused: the cover dimmed, or a dark key, with a pause sign.
// - stopped (nothing playing, or the player is not running): a dark key with a dim play sign.
ObjC.import('AppKit');

var SIDE = 72;

function colour(r, g, b, a) {
  return $.NSColor.colorWithSRGBRedGreenBlueAlpha(r, g, b, a);
}

function rect(x, y, w, h) {
  return $.NSMakeRect(x, y, w, h);
}

function rounded(x, y, w, h, radius) {
  $.NSBezierPath.bezierPathWithRoundedRectXRadiusYRadius(rect(x, y, w, h), radius, radius).fill;
}

// Draws the image as large as it covers the key, cropping the longer side, centred.
function cover(image) {
  var size = image.size;
  var side = Math.min(size.width, size.height);
  if (side <= 0) return false;
  var from = rect((size.width - side) / 2, (size.height - side) / 2, side, side);
  image.drawInRectFromRectOperationFraction(rect(0, 0, SIDE, SIDE), from, $.NSCompositingOperationSourceOver, 1);
  return true;
}

function pauseSign(alpha) {
  colour(1, 1, 1, alpha).setFill;
  rounded(23, 22, 9, 28, 2);
  rounded(40, 22, 9, 28, 2);
}

function playSign(alpha) {
  colour(1, 1, 1, alpha).setFill;
  var path = $.NSBezierPath.bezierPath;
  path.moveToPoint($.NSMakePoint(27, 21));
  path.lineToPoint($.NSMakePoint(27, 51));
  path.lineToPoint($.NSMakePoint(52, 36));
  path.closePath;
  path.fill;
}

function bars() {
  colour(1, 1, 1, 0.9).setFill;
  rounded(20, 22, 8, 22, 2);
  rounded(32, 22, 8, 30, 2);
  rounded(44, 22, 8, 16, 2);
}

function run(argv) {
  var out = argv[0];
  var state = argv[1];
  var artwork = argv.length > 2 && argv[2] ? $.NSImage.alloc.initWithContentsOfFile(argv[2]) : null;
  if (artwork && artwork.isNil()) artwork = null;

  var rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
    null, SIDE, SIDE, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  var context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext(context);
  context.setImageInterpolation($.NSImageInterpolationHigh);

  colour(0.09, 0.09, 0.12, 1).setFill;
  $.NSRectFill(rect(0, 0, SIDE, SIDE));
  var drawn = state !== 'stopped' && artwork !== null && cover(artwork);

  if (state === 'playing') {
    if (!drawn) bars();
  } else if (state === 'paused') {
    if (drawn) {
      colour(0, 0, 0, 0.55).setFill;
      $.NSRectFillUsingOperation(rect(0, 0, SIDE, SIDE), $.NSCompositingOperationSourceOver);
    }
    pauseSign(0.95);
  } else {
    playSign(0.45);
  }

  context.flushGraphics;
  $.NSGraphicsContext.restoreGraphicsState;
  var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $());
  if (!png.writeToFileAtomically(out, true)) throw new Error('could not write ' + out);
  return '';
}
