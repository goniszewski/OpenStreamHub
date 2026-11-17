// Draws one 72x72 key for the Claude Code plugin with AppKit and writes it as a PNG.
// Run as: osascript -l JavaScript render.js <out.png> <face...>, where face is one of
// - waiting|done|working|idle <project> <detail> <others>: a session. Others, when set, is how many other sessions
//   need you (waiting or done), shown as a badge.
// - none: no session.
// - meter <percent> <cost> <project>: a ring of how full the context window is, with the cost below.
ObjC.import('AppKit');

var SIDE = 72;

function colour(r, g, b, a) {
  return $.NSColor.colorWithSRGBRedGreenBlueAlpha(r, g, b, a);
}

function rect(x, y, w, h) {
  return $.NSMakeRect(x, y, w, h);
}

// Draws one line centred in the box, shrinking it down to two points smaller to fit, then cutting it with an ellipsis.
function text(value, x, y, w, h, size, bold, fill) {
  if (!value) return;
  var style = $.NSMutableParagraphStyle.alloc.init;
  style.alignment = $.NSTextAlignmentCenter;
  style.lineBreakMode = $.NSLineBreakByTruncatingTail;
  var string = $.NSString.alloc.initWithUTF8String(value);
  var attributes;
  for (var fit = size; fit >= size - 2; fit--) {
    var font = bold ? $.NSFont.boldSystemFontOfSize(fit) : $.NSFont.systemFontOfSize(fit);
    attributes = $.NSDictionary.dictionaryWithObjectsForKeys(
      $([font, fill, style]),
      $([$.NSFontAttributeName, $.NSForegroundColorAttributeName, $.NSParagraphStyleAttributeName]));
    if (string.sizeWithAttributes(attributes).width <= w) break;
  }
  string.drawInRectWithAttributes(rect(x, y, w, h), attributes);
}

function circle(x, y, radius) {
  $.NSBezierPath.bezierPathWithOvalInRect(rect(x - radius, y - radius, radius * 2, radius * 2)).fill;
}

function dots(fill) {
  fill.setFill;
  circle(24, 36, 4);
  circle(36, 36, 4);
  circle(48, 36, 4);
}

function check(stroke) {
  stroke.setStroke;
  var path = $.NSBezierPath.bezierPath;
  path.lineWidth = 6;
  path.lineCapStyle = $.NSLineCapStyleRound;
  path.lineJoinStyle = $.NSLineJoinStyleRound;
  path.moveToPoint($.NSMakePoint(23, 37));
  path.lineToPoint($.NSMakePoint(32, 28));
  path.lineToPoint($.NSMakePoint(50, 46));
  path.stroke;
}

function badge(count) {
  if (!count) return;
  colour(0.95, 0.35, 0.25, 1).setFill;
  circle(62, 62, 8);
  text(count.length > 1 ? '9+' : count, 54, 55, 16, 14, 10, true, colour(1, 1, 1, 1));
}

function session(state, project, detail, others) {
  var background, ink, dim;
  if (state === 'waiting') {
    background = colour(0.93, 0.62, 0.15, 1);
    ink = colour(0.1, 0.07, 0.02, 1);
    dim = colour(0.1, 0.07, 0.02, 0.8);
  } else if (state === 'done') {
    background = colour(0.16, 0.55, 0.32, 1);
    ink = colour(1, 1, 1, 1);
    dim = colour(1, 1, 1, 0.8);
  } else if (state === 'working') {
    background = colour(0.11, 0.15, 0.27, 1);
    ink = colour(1, 1, 1, 1);
    dim = colour(0.7, 0.8, 1, 0.85);
  } else {
    background = colour(0.11, 0.11, 0.13, 1);
    ink = colour(1, 1, 1, 0.75);
    dim = colour(1, 1, 1, 0.5);
  }
  background.setFill;
  $.NSRectFill(rect(0, 0, SIDE, SIDE));
  text(project || 'Claude', 3, 53, others ? 52 : 66, 16, 11, true, ink);
  if (state === 'waiting') {
    text('?', 0, 18, SIDE, 36, 30, true, ink);
  } else if (state === 'done') {
    check(ink);
  } else if (state === 'working') {
    dots(dim);
  } else {
    colour(1, 1, 1, 0.35).setFill;
    $.NSBezierPath.bezierPathWithRoundedRectXRadiusYRadius(rect(24, 34, 24, 4), 2, 2).fill;
  }
  var label = state === 'waiting' ? (detail || 'needs you') : state === 'done' ? 'done' :
    state === 'working' ? (detail || 'working') : 'idle';
  text(label, 2, 3, 68, 15, 10, state === 'waiting', dim);
  badge(others);
}

function nothing() {
  colour(0.07, 0.07, 0.09, 1).setFill;
  $.NSRectFill(rect(0, 0, SIDE, SIDE));
  text('Claude', 2, 38, 68, 16, 11, true, colour(1, 1, 1, 0.45));
  text('no session', 2, 20, 68, 15, 10, false, colour(1, 1, 1, 0.35));
}

function meter(percent, cost, project) {
  colour(0.07, 0.07, 0.09, 1).setFill;
  $.NSRectFill(rect(0, 0, SIDE, SIDE));
  var centre = $.NSMakePoint(36, 42);
  var ring = $.NSBezierPath.bezierPath;
  ring.lineWidth = 6;
  ring.appendBezierPathWithArcWithCenterRadiusStartAngleEndAngle(centre, 21, 0, 360);
  colour(1, 1, 1, 0.12).setStroke;
  ring.stroke;
  if (percent !== '') {
    var share = Math.max(0, Math.min(100, Number(percent))) / 100;
    var fill = share < 0.6 ? colour(0.3, 0.75, 0.45, 1) : share < 0.85 ? colour(0.95, 0.65, 0.2, 1) :
      colour(0.95, 0.3, 0.25, 1);
    if (share > 0) {
      var arc = $.NSBezierPath.bezierPath;
      arc.lineWidth = 6;
      arc.lineCapStyle = $.NSLineCapStyleRound;
      arc.appendBezierPathWithArcWithCenterRadiusStartAngleEndAngleClockwise(centre, 21, 90, 90 - 360 * share, true);
      fill.setStroke;
      arc.stroke;
    }
    text(percent + '%', 12, 34, 48, 17, 13, true, colour(1, 1, 1, 0.95));
  } else {
    text(project ? '–' : 'no session', 12, 35, 48, 15, project ? 13 : 8, false, colour(1, 1, 1, 0.5));
  }
  text(cost || project, 2, 2, 68, 15, 10, false, colour(1, 1, 1, 0.7));
}

function run(argv) {
  var out = argv[0];
  var face = argv[1];
  var rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
    null, SIDE, SIDE, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  var context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext(context);
  context.setImageInterpolation($.NSImageInterpolationHigh);

  if (face === 'meter') meter(argv[2] || '', argv[3] || '', argv[4] || '');
  else if (face === 'none') nothing();
  else session(face, argv[2] || '', argv[3] || '', argv[4] || '');

  context.flushGraphics;
  $.NSGraphicsContext.restoreGraphicsState;
  var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $());
  if (!png.writeToFileAtomically(out, true)) throw new Error('could not write ' + out);
  return '';
}
