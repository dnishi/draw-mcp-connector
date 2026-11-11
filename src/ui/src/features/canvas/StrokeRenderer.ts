import { FLOATS_PER_VERTEX } from './strokeMesh';
import type { View } from './view';

// TypeScript 7's lib.dom.d.ts does not declare GPUBufferUsage / GPUTextureUsage.
// They exist in browsers (namespaces in the WebGPU spec, https://www.w3.org/TR/webgpu/), so only their types are declared
declare const GPUBufferUsage: {
  readonly COPY_DST: number;
  readonly VERTEX: number;
  readonly UNIFORM: number;
};
declare const GPUTextureUsage: {
  readonly RENDER_ATTACHMENT: number;
};
declare const GPUShaderStage: {
  readonly VERTEX: number;
  readonly FRAGMENT: number;
};

/** Multisample count to reduce jagged line edges */
const SAMPLE_COUNT = 4;
const BYTES_PER_VERTEX = FLOATS_PER_VERTEX * Float32Array.BYTES_PER_ELEMENT;
const MIN_VERTEX_BUFFER_BYTES = 64 * 1024;
const WHITE = { r: 1, g: 1, b: 1, a: 1 };
/** Grid spacing (canvas CSS pixels) */
const GRID_SPACING = 100;
/** Dash and gap length of the dashed grid lines (CSS pixels on the view; written as integers) */
const GRID_DASH = 4;
const GRID_GAP = 4;
/** Size of the shader's Viewport (six values: size, origin, scale, pixelRatio, rounded up to a multiple of 16 bytes) */
const VIEWPORT_BYTES = 8 * Float32Array.BYTES_PER_ELEMENT;
/** Offset of origin within Viewport (after size) */
const VIEW_OFFSET_BYTES = 2 * Float32Array.BYTES_PER_ELEMENT;
/** Offset of pixelRatio within Viewport (after scale) */
const PIXEL_RATIO_OFFSET_BYTES = 5 * Float32Array.BYTES_PER_ELEMENT;

// Vertices are passed in canvas coordinates (CSS pixels); the shader zooms and pans them, then
// converts to clip coordinates (-1 at the bottom left, 1 at the top right). Zooming needs no vertex rebuild
const SHADER = /* wgsl */ `
struct Viewport {
  // View size (CSS pixels)
  size: vec2f,
  // Canvas point at the view's top left (View in view.ts)
  origin: vec2f,
  scale: f32,
  // Render pixels per CSS pixel (devicePixelRatio)
  pixelRatio: f32,
};
@group(0) @binding(0) var<uniform> viewport: Viewport;

struct VertexOutput {
  @builtin(position) position: vec4f,
  @location(0) color: vec3f,
};

@vertex
fn vs(@location(0) position: vec2f, @location(1) color: vec3f) -> VertexOutput {
  let screen = (position - viewport.origin) * viewport.scale;
  let clip = screen / viewport.size * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
  return VertexOutput(vec4f(clip, 0.0, 1.0), color);
}

// Stroke color is per vertex (all vertices of a stroke share the same color)
@fragment
fn fs(@location(0) color: vec3f) -> @location(0) vec4f {
  return vec4f(color, 1.0);
}

// Grid: draw one triangle covering the whole view, and color each pixel by its distance to the nearest grid line.
// Spacing is in canvas coordinates (it widens when zooming in); line width stays 1 CSS pixel on the view at any zoom
const GRID_SPACING = ${GRID_SPACING}.0;
// Very light gray (#eeeeee)
const GRID_COLOR = vec3f(0.933, 0.933, 0.933);
// Dash and gap length (CSS pixels on the view). Like the line width, unchanged by zoom
const GRID_DASH = ${GRID_DASH}.0;
const GRID_GAP = ${GRID_GAP}.0;

@vertex
fn vsGrid(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
  // The index is decided at run time, so keep it in a var
  var corners = array(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(corners[index], 0.0, 1.0);
}

// How much a band of the given width covers a pixel offset render pixels from its center. The edge is blurred over 1 pixel
fn bandCoverage(offset: f32, width: f32) -> f32 {
  return clamp(width * 0.5 + 0.5 - offset, 0.0, 1.0);
}

// Whether the point that is along render pixels from a crossing, measured along the line, is on a dash.
// Dashes are centered on crossings so crossings look like "+" at any zoom
fn dashCoverage(along: f32) -> f32 {
  let period = (GRID_DASH + GRID_GAP) * viewport.pixelRatio;
  let fromDashCenter = abs(along - round(along / period) * period);
  return bandCoverage(fromDashCenter, GRID_DASH * viewport.pixelRatio);
}

@fragment
fn fsGrid(@builtin(position) position: vec4f) -> @location(0) vec4f {
  // position.xy is in render pixels. One canvas CSS pixel is pixelsPerUnit render pixels
  let pixelsPerUnit = viewport.scale * viewport.pixelRatio;
  let canvasPoint = viewport.origin + position.xy / pixelsPerUnit;
  // Distance to the nearest line, horizontally and vertically (render pixels).
  // For a vertical line (x is near), the y distance is the distance along the line from the nearest crossing (and vice versa for horizontal lines)
  let lineDistance = abs(fract(canvasPoint / GRID_SPACING + 0.5) - 0.5) * GRID_SPACING * pixelsPerUnit;
  let lineWidth = viewport.pixelRatio;
  let vertical = bandCoverage(lineDistance.x, lineWidth) * dashCoverage(lineDistance.y);
  let horizontal = bandCoverage(lineDistance.y, lineWidth) * dashCoverage(lineDistance.x);
  let coverage = max(vertical, horizontal);
  return vec4f(mix(vec3f(1.0), GRID_COLOR, coverage), 1.0);
}
`;

export async function requestGpuDevice(): Promise<GPUDevice> {
  // navigator.gpu exists only on HTTPS (or localhost) pages
  if (!navigator.gpu) {
    throw new Error(
      "WebGPU isn't available. Open this page over HTTPS in a browser that supports WebGPU.",
    );
  }
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error("Couldn't get a WebGPU adapter.");
  return adapter.requestDevice();
}

/** Draws stroke triangles onto the canvas with WebGPU */
export class StrokeRenderer {
  readonly #canvas: HTMLCanvasElement;
  readonly #device: GPUDevice;
  readonly #context: GPUCanvasContext;
  readonly #format: GPUTextureFormat;
  readonly #pipeline: GPURenderPipeline;
  readonly #gridPipeline: GPURenderPipeline;
  readonly #viewportBuffer: GPUBuffer;
  readonly #bindGroup: GPUBindGroup;
  #vertexBuffer: GPUBuffer | null = null;
  #msaaTexture: GPUTexture | null = null;

  constructor(canvas: HTMLCanvasElement, device: GPUDevice) {
    const context = canvas.getContext('webgpu');
    if (!(context instanceof GPUCanvasContext)) {
      throw new Error("Couldn't get a WebGPU context from the canvas.");
    }

    this.#canvas = canvas;
    this.#device = device;
    this.#context = context;
    this.#format = navigator.gpu.getPreferredCanvasFormat();
    context.configure({ device, format: this.#format, alphaMode: 'opaque' });

    const module = device.createShaderModule({ code: SHADER });
    // Strokes and the grid share the same Viewport, so create a shared layout instead of layout: 'auto'
    const bindGroupLayout = device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: 'uniform' },
        },
      ],
    });
    const layout = device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout],
    });
    const fragmentTargets = [{ format: this.#format }];
    const multisample = { count: SAMPLE_COUNT };

    this.#pipeline = device.createRenderPipeline({
      layout,
      vertex: {
        module,
        entryPoint: 'vs',
        buffers: [
          {
            arrayStride: BYTES_PER_VERTEX,
            attributes: [
              // x, y (canvas coordinates)
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              // r, g, b (0 to 1)
              {
                shaderLocation: 1,
                offset: 2 * Float32Array.BYTES_PER_ELEMENT,
                format: 'float32x3',
              },
            ],
          },
        ],
      },
      fragment: { module, entryPoint: 'fs', targets: fragmentTargets },
      primitive: { topology: 'triangle-list' },
      multisample,
    });
    this.#gridPipeline = device.createRenderPipeline({
      layout,
      vertex: { module, entryPoint: 'vsGrid' },
      fragment: { module, entryPoint: 'fsGrid', targets: fragmentTargets },
      primitive: { topology: 'triangle-list' },
      multisample,
    });

    this.#viewportBuffer = device.createBuffer({
      size: VIEWPORT_BYTES,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.#bindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [{ binding: 0, resource: { buffer: this.#viewportBuffer } }],
    });
  }

  /** Changes the render resolution to match the display size (CSS pixels) */
  resize(cssWidth: number, cssHeight: number, devicePixelRatio: number): void {
    this.#canvas.width = Math.max(1, Math.round(cssWidth * devicePixelRatio));
    this.#canvas.height = Math.max(1, Math.round(cssHeight * devicePixelRatio));
    this.#device.queue.writeBuffer(
      this.#viewportBuffer,
      0,
      new Float32Array([cssWidth, cssHeight]),
    );
    this.#device.queue.writeBuffer(
      this.#viewportBuffer,
      PIXEL_RATIO_OFFSET_BYTES,
      new Float32Array([devicePixelRatio]),
    );

    this.#msaaTexture?.destroy();
    this.#msaaTexture = this.#device.createTexture({
      size: [this.#canvas.width, this.#canvas.height],
      format: this.#format,
      sampleCount: SAMPLE_COUNT,
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
  }

  /**
   * Redraws vertices (triangle-list, canvas coordinates) over the whole view with the view's zoom and position.
   * If showGrid, draws the grid under the strokes
   */
  draw(vertices: Float32Array, view: View, showGrid: boolean): void {
    if (!this.#msaaTexture) return;

    this.#device.queue.writeBuffer(
      this.#viewportBuffer,
      VIEW_OFFSET_BYTES,
      new Float32Array([view.originX, view.originY, view.scale]),
    );

    const vertexCount = vertices.length / FLOATS_PER_VERTEX;
    const vertexBuffer = vertexCount > 0 ? this.#writeVertices(vertices) : null;

    const encoder = this.#device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.#msaaTexture.createView(),
          resolveTarget: this.#context.getCurrentTexture().createView(),
          clearValue: WHITE,
          loadOp: 'clear',
          storeOp: 'discard',
        },
      ],
    });
    pass.setBindGroup(0, this.#bindGroup);
    if (showGrid) {
      pass.setPipeline(this.#gridPipeline);
      pass.draw(3);
    }
    if (vertexBuffer) {
      pass.setPipeline(this.#pipeline);
      pass.setVertexBuffer(0, vertexBuffer, 0, vertices.byteLength);
      pass.draw(vertexCount);
    }
    pass.end();
    this.#device.queue.submit([encoder.finish()]);
  }

  destroy(): void {
    this.#device.destroy();
  }

  /** Writes to the vertex buffer. If it is too small, recreates it, doubling the size as needed */
  #writeVertices(vertices: Float32Array): GPUBuffer {
    if (!this.#vertexBuffer || this.#vertexBuffer.size < vertices.byteLength) {
      let size = this.#vertexBuffer?.size ?? MIN_VERTEX_BUFFER_BYTES;
      while (size < vertices.byteLength) size *= 2;

      this.#vertexBuffer?.destroy();
      this.#vertexBuffer = this.#device.createBuffer({
        size,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
    }
    this.#device.queue.writeBuffer(this.#vertexBuffer, 0, vertices);
    return this.#vertexBuffer;
  }
}
