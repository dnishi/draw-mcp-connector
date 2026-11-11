import { strokeColor, toRgb } from './color';
import {
  radiusAt,
  smoothPoints,
  type Stroke,
  type StrokePoint,
} from './stroke';

/** Floats per vertex (x, y and color r, g, b) */
export const FLOATS_PER_VERTEX = 5;

type Rgb = readonly [number, number, number];

/** Where vertices are written. Color is the same for the whole stroke, so it is kept here */
interface MeshWriter {
  out: number[];
  rgb: Rgb;
}

function pushVertex({ out, rgb }: MeshWriter, x: number, y: number): void {
  out.push(x, y, ...rgb);
}

/** Number of triangles approximating the circle placed at each point */
const CIRCLE_SEGMENTS = 12;
const UNIT_CIRCLE = Array.from({ length: CIRCLE_SEGMENTS + 1 }, (_, i) => {
  const angle = (i / CIRCLE_SEGMENTS) * Math.PI * 2;
  return { cos: Math.cos(angle), sin: Math.sin(angle) };
});

function pushDisc(mesh: MeshWriter, { x, y, pressure }: StrokePoint): void {
  const r = radiusAt(pressure);
  for (let i = 0; i < CIRCLE_SEGMENTS; i++) {
    const a = UNIT_CIRCLE[i];
    const b = UNIT_CIRCLE[i + 1];
    pushVertex(mesh, x, y);
    pushVertex(mesh, x + a.cos * r, y + a.sin * r);
    pushVertex(mesh, x + b.cos * r, y + b.sin * r);
  }
}

/** Connects two points with a quad (two triangles) whose width follows each point's pressure */
function pushSegment(mesh: MeshWriter, a: StrokePoint, b: StrokePoint): void {
  const length = Math.hypot(b.x - a.x, b.y - a.y);
  if (length === 0) return;

  const nx = -(b.y - a.y) / length;
  const ny = (b.x - a.x) / length;
  const ra = radiusAt(a.pressure);
  const rb = radiusAt(b.pressure);
  const a1 = [a.x + nx * ra, a.y + ny * ra] as const;
  const a2 = [a.x - nx * ra, a.y - ny * ra] as const;
  const b1 = [b.x + nx * rb, b.y + ny * rb] as const;
  const b2 = [b.x - nx * rb, b.y - ny * rb] as const;
  for (const [x, y] of [a1, a2, b1, b2, b1, a2]) pushVertex(mesh, x, y);
}

/**
 * Turns a stroke into triangle-list vertices: a circle at each point and a quad between points.
 * color is "#rrggbb"
 */
export function tessellateStroke(
  points: readonly StrokePoint[],
  color: string,
): Float32Array {
  const smoothed = smoothPoints(points);
  const mesh: MeshWriter = { out: [], rgb: toRgb(color) };
  smoothed.forEach((point, i) => {
    pushDisc(mesh, point);
    if (i > 0) pushSegment(mesh, smoothed[i - 1], point);
  });
  return new Float32Array(mesh.out);
}

// Finished strokes never change (undo etc. rebuild the array), so results can be reused per object
const meshCache = new WeakMap<Stroke, Float32Array>();

function strokeMesh(stroke: Stroke): Float32Array {
  let mesh = meshCache.get(stroke);
  if (!mesh) {
    mesh = tessellateStroke(stroke.points, strokeColor(stroke));
    meshCache.set(stroke, mesh);
  }
  return mesh;
}

/** The stroke being drawn */
export interface ActiveStroke {
  points: readonly StrokePoint[];
  color: string;
}

/** Collects the vertices of every stroke drawn on screen into one array */
export function buildSceneVertices(
  strokes: Iterable<Stroke>,
  active?: ActiveStroke,
): Float32Array {
  const meshes = [...strokes].map(strokeMesh);
  if (active && active.points.length > 0) {
    meshes.push(tessellateStroke(active.points, active.color));
  }

  const vertices = new Float32Array(
    meshes.reduce((sum, mesh) => sum + mesh.length, 0),
  );
  let offset = 0;
  for (const mesh of meshes) {
    vertices.set(mesh, offset);
    offset += mesh.length;
  }
  return vertices;
}
