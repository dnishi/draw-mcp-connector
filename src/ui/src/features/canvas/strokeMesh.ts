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
  if (length === 0) {
    return;
  }

  const nx = -(b.y - a.y) / length;
  const ny = (b.x - a.x) / length;
  const ra = radiusAt(a.pressure);
  const rb = radiusAt(b.pressure);
  const a1 = [a.x + nx * ra, a.y + ny * ra] as const;
  const a2 = [a.x - nx * ra, a.y - ny * ra] as const;
  const b1 = [b.x + nx * rb, b.y + ny * rb] as const;
  const b2 = [b.x - nx * rb, b.y - ny * rb] as const;
  for (const [x, y] of [a1, a2, b1, b2, b1, a2]) {
    pushVertex(mesh, x, y);
  }
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
    if (i > 0) {
      pushSegment(mesh, smoothed[i - 1], point);
    }
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

/** How to apply the vertices of committed strokes to the GPU buffer */
export type CommittedVertices =
  | { kind: 'unchanged' }
  /** Strokes were added after the previous ones. Write data at offset (in floats) */
  | { kind: 'append'; offset: number; data: Float32Array }
  /** Any other change (undo, eraser, sheet switch, changes from the PC, etc.). Rewrite everything */
  | { kind: 'rebuild'; data: Float32Array };

/** Concatenates vertex arrays */
function concatMeshes(meshes: readonly Float32Array[]): Float32Array {
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

/** Remembers the previous list of strokes and builds CommittedVertices from the difference with the next one */
export class CommittedStrokes {
  /** The previous list. null before the first call (the first call is a rebuild) */
  #previous: readonly Stroke[] | null = null;
  /** Number of floats of vertices in the previous list */
  #length = 0;

  update(strokes: readonly Stroke[]): CommittedVertices {
    const previous = this.#previous;
    this.#previous = strokes;
    if (previous === strokes) {
      return { kind: 'unchanged' };
    }
    // append if the new list starts with the previous one (same order, same objects) and only grew at the end
    const appended =
      previous !== null &&
      strokes.length >= previous.length &&
      previous.every((stroke, i) => strokes[i] === stroke);
    if (!appended) {
      const data = concatMeshes(strokes.map(strokeMesh));
      this.#length = data.length;
      return { kind: 'rebuild', data };
    }
    const data = concatMeshes(strokes.slice(previous.length).map(strokeMesh));
    if (data.length === 0) {
      return { kind: 'unchanged' };
    }
    const offset = this.#length;
    this.#length += data.length;
    return { kind: 'append', offset, data };
  }

  /** Current number of floats of vertices */
  get length(): number {
    return this.#length;
  }
}
