import { FLOATS_PER_VERTEX, type CommittedVertices } from './strokeMesh';
import type { View } from './view';

// TypeScript 7's lib.dom.d.ts does not declare GPUBufferUsage / GPUTextureUsage.
// They exist in browsers (namespaces in the WebGPU spec, https://www.w3.org/TR/webgpu/), so only their types are declared
declare const GPUBufferUsage: {
  readonly COPY_SRC: number;
  readonly COPY_DST: number;
  readonly VERTEX: number;
  readonly UNIFORM: number;
};
declare const GPUTextureUsage: {
  readonly RENDER_ATTACHMENT: number;
  readonly TEXTURE_BINDING: number;
  readonly COPY_DST: number;
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
/** Position of lod in Viewport (after pixelRatio) */
const IMAGE_LOD_OFFSET_BYTES = 6 * Float32Array.BYTES_PER_ELEMENT;
/** Format of the image texture (also the render target when building the mip chain) */
const IMAGE_TEXTURE_FORMAT: GPUTextureFormat = 'rgba8unorm';

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
  // Mip level of the image to read (passed from the CPU in draw(); 0 when enlarged)
  lod: f32,
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

// Image: stretched over the placement rectangle (under the strokes). Alpha is blended, so transparent parts show what is below
@group(0) @binding(1) var imageTexture: texture_2d<f32>;
@group(0) @binding(2) var imageSampler: sampler;

struct ImageVertexOutput {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vsImage(@location(0) position: vec2f, @location(1) uv: vec2f) -> ImageVertexOutput {
  let screen = (position - viewport.origin) * viewport.scale;
  let clip = screen / viewport.size * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
  return ImageVertexOutput(vec4f(clip, 0.0, 1.0), uv);
}

// textureSampleLevel instead of textureSample: implicit LOD is not available in a multisampled pass.
// The level to read is computed on the CPU from how many screen pixels one image pixel becomes (viewport.lod).
// When shrunk it reads from the mip chain (#buildMipLevels), so thin lines do not break up or flicker
@fragment
fn fsImage(@location(0) uv: vec2f) -> @location(0) vec4f {
  return textureSampleLevel(imageTexture, imageSampler, uv, viewport.lod);
}

// Building the mip chain: draws the half-size image of the level above, stretched to fill the target level
// (a 2×2 average with the linear filter). WebGPU has no function to build mip chains, so
// each level is drawn by our own code (https://www.w3.org/TR/webgpu/)
@group(0) @binding(0) var mipSource: texture_2d<f32>;
@group(0) @binding(1) var mipSampler: sampler;

struct MipVertexOutput {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vsMip(@builtin(vertex_index) index: u32) -> MipVertexOutput {
  // The corners of a triangle covering the whole screen point at (0, 0) to (1, 1) of mipSource
  // (y=1 in clip space is the top of the screen = row 0 of the texture)
  var corners = array(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  let corner = corners[index];
  let uv = corner * vec2f(0.5, -0.5) + vec2f(0.5, 0.5);
  return MipVertexOutput(vec4f(corner, 0.0, 1.0), uv);
}

@fragment
fn fsMip(@location(0) uv: vec2f) -> @location(0) vec4f {
  return textureSampleLevel(mipSource, mipSampler, uv, 0.0);
}

// Grid: draws one triangle covering the whole screen and colors each pixel by its distance to the nearest grid line.
// The spacing is in canvas coordinates (widens when zoomed in); the line width stays 1 CSS pixel on screen when zoomed
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

/**
 * Message shown in browsers that can't use WebGPU. It avoids technical terms and lists only supported browsers and versions.
 * Each version is the first one with both WebGPU and WebTransport serverCertificateHashes
 * (MDN browser-compat-data, api/GPU.json and api/WebTransport.json, as of 2026-11-15).
 * Firefox isn't listed: its WebGPU support is partial and depends on the OS, and it hasn't been tested
 */
export const UNSUPPORTED_BROWSER_MESSAGE = [
  "This browser isn't supported. Please use one of these:",
  '- Safari 26.4 or later (iPad, iPhone, Mac)',
  '- Chrome or Edge 113 or later (Windows, Mac)',
  '- Chrome 121 or later (Android)',
].join('\n');

export async function requestGpuDevice(): Promise<GPUDevice> {
  // navigator.gpu exists only on HTTPS (or localhost) pages
  if (!navigator.gpu) {
    throw new Error(UNSUPPORTED_BROWSER_MESSAGE);
  }
  // Not available where the GPU can't be used (e.g. when disabled, or for search engine crawlers)
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) {
    throw new Error(UNSUPPORTED_BROWSER_MESSAGE);
  }
  const device = await adapter.requestDevice();
  // Validation errors (such as binding mismatches, where commands are discarded) appear in the console
  // once a handler is attached. This makes silent drawing bugs traceable during development
  device.onuncapturederror = (event: GPUUncapturedErrorEvent) => {
    console.error(
      `[WebGPU] ${event.error.constructor.name}: ${event.error.message}`,
    );
  };
  return device;
}

/** Image laid underneath: the decoded bitmap and its placement on the canvas */
export interface ImageUnderlay {
  bitmap: ImageBitmap;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** One image vertex: x, y (canvas coordinates) and u, v (position within the image) */
const FLOATS_PER_IMAGE_VERTEX = 4;
const IMAGE_VERTEX_COUNT = 6;

/** Draws stroke triangles onto the canvas with WebGPU */
export class StrokeRenderer {
  readonly #canvas: HTMLCanvasElement;
  readonly #device: GPUDevice;
  readonly #context: GPUCanvasContext;
  readonly #format: GPUTextureFormat;
  readonly #pipeline: GPURenderPipeline;
  readonly #gridPipeline: GPURenderPipeline;
  readonly #imagePipeline: GPURenderPipeline;
  readonly #mipPipeline: GPURenderPipeline;
  readonly #sampler: GPUSampler;
  readonly #viewportBuffer: GPUBuffer;
  readonly #bindGroup: GPUBindGroup;
  // Vertices of committed strokes change only on append or rebuild, so they are kept
  // apart from the buffer of the stroke being drawn (rewritten every frame)
  #committedVertexBuffer: GPUBuffer | null = null;
  /** Number of floats of vertices in the committed strokes buffer */
  #committedVertexFloats = 0;
  /** Buffer of the stroke being drawn */
  #activeVertexBuffer: GPUBuffer | null = null;
  #msaaTexture: GPUTexture | null = null;
  #devicePixelRatio = 1;
  #imageBitmap: ImageBitmap | null = null;
  #imageTexture: GPUTexture | null = null;
  #imageBindGroup: GPUBindGroup | null = null;
  #imageVertexBuffer: GPUBuffer | null = null;

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
    // The image uses a texture and a sampler in addition to Viewport, so it gets its own layout
    this.#imagePipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'vsImage',
        buffers: [
          {
            arrayStride:
              FLOATS_PER_IMAGE_VERTEX * Float32Array.BYTES_PER_ELEMENT,
            attributes: [
              // x, y (canvas coordinates)
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              // u, v (position within the image)
              {
                shaderLocation: 1,
                offset: 2 * Float32Array.BYTES_PER_ELEMENT,
                format: 'float32x2',
              },
            ],
          },
        ],
      },
      fragment: {
        module,
        entryPoint: 'fsImage',
        targets: [
          {
            format: this.#format,
            blend: {
              color: {
                // The texture holds colors premultiplied by alpha (premultipliedAlpha in
                // #syncImageTexture). Linear interpolation without premultiplying bleeds the color of
                // transparent parts into the edges
                srcFactor: 'one',
                dstFactor: 'one-minus-src-alpha',
              },
              alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
            },
          },
        ],
      },
      // The default cullMode in the spec (W3C WebGPU) is "none", but this is a quad, so state
      // explicitly that it is not culled (so it does not vanish if implementations differ or the default changes)
      primitive: { topology: 'triangle-list', cullMode: 'none' },
      multisample,
    });
    // Building the mip chain: reads the texture of the level above and writes into the half-size level.
    // Colors are written as is (the premultiplied colors in the texture are not diluted), so there is no blending
    this.#mipPipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex: { module, entryPoint: 'vsMip' },
      fragment: {
        module,
        entryPoint: 'fsMip',
        targets: [{ format: IMAGE_TEXTURE_FORMAT }],
      },
      primitive: { topology: 'triangle-list' },
    });
    // mipmapFilter: when shrinking an image, also blends linearly between the two levels adjacent to the computed level (lod).
    // Building the mip chain (#mipPipeline) uses the same sampler (it only reads views narrowed
    // to one level, so mipmapFilter has no effect)
    this.#sampler = device.createSampler({
      magFilter: 'linear',
      minFilter: 'linear',
      mipmapFilter: 'linear',
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
    this.#devicePixelRatio = devicePixelRatio;
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
   * Redraws the whole screen with the committed strokes (committed) and the stroke being drawn (active, triangle-list, canvas coordinates)
   * at the zoom and position of view.
   * Order: white background → grid → image (image, below the strokes) → committed strokes → stroke being drawn
   */
  draw(
    committed: CommittedVertices,
    active: Float32Array,
    view: View,
    image?: ImageUnderlay,
  ): void {
    // Apply changes to the committed strokes to the buffer even before drawing is ready (msaaTexture).
    // committed is the difference from the previous frame, so dropping it would leave later frames without those strokes
    const committedVertexCount = this.#updateCommittedVertices(committed);
    if (!this.#msaaTexture) {
      return;
    }

    this.#device.queue.writeBuffer(
      this.#viewportBuffer,
      VIEW_OFFSET_BYTES,
      new Float32Array([view.originX, view.originY, view.scale]),
    );

    if (image) {
      this.#syncImageTexture(image.bitmap);
      // How many drawing pixels one image pixel becomes (placement width × display zoom × devicePixelRatio).
      // Read from the mip chain by the amount it shrinks below 1 (log2). 0 (level 0) when enlarged
      const screenPerSourcePixel =
        (image.width * view.scale * this.#devicePixelRatio) /
        image.bitmap.width;
      const lod = Math.max(0, -Math.log2(screenPerSourcePixel));
      this.#device.queue.writeBuffer(
        this.#viewportBuffer,
        IMAGE_LOD_OFFSET_BYTES,
        new Float32Array([lod]),
      );
    }
    const committedBuffer = this.#committedVertexBuffer;
    const activeVertexCount = active.length / FLOATS_PER_VERTEX;
    const activeBuffer =
      activeVertexCount > 0 ? this.#writeActiveVertices(active) : null;

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
    pass.setPipeline(this.#gridPipeline);
    pass.draw(3);
    if (image && this.#imageTexture && this.#imageBindGroup) {
      pass.setPipeline(this.#imagePipeline);
      pass.setBindGroup(0, this.#imageBindGroup);
      pass.setVertexBuffer(
        0,
        this.#writeImageQuad(image),
        0,
        IMAGE_VERTEX_COUNT *
          FLOATS_PER_IMAGE_VERTEX *
          Float32Array.BYTES_PER_ELEMENT,
      );
      pass.draw(IMAGE_VERTEX_COUNT);
    }
    // The spec checks binding consistency at draw time, but to avoid setting the shared bind group
    // (binding 0 only) under the image pipeline (which has 3 bindings with layout: 'auto'),
    // switch the pipeline first and then set the bind group
    pass.setPipeline(this.#pipeline);
    pass.setBindGroup(0, this.#bindGroup);
    if (committedBuffer && committedVertexCount > 0) {
      pass.setVertexBuffer(
        0,
        committedBuffer,
        0,
        committedVertexCount * BYTES_PER_VERTEX,
      );
      pass.draw(committedVertexCount);
    }
    if (activeBuffer) {
      pass.setVertexBuffer(
        0,
        activeBuffer,
        0,
        activeVertexCount * BYTES_PER_VERTEX,
      );
      pass.draw(activeVertexCount);
    }
    pass.end();
    this.#device.queue.submit([encoder.finish()]);
  }

  destroy(): void {
    this.#imageTexture?.destroy();
    this.#device.destroy();
  }

  /** Reloads only a replaced image into the GPU texture (including the mip chain) */
  #syncImageTexture(bitmap: ImageBitmap): void {
    if (this.#imageBitmap === bitmap && this.#imageTexture) {
      return;
    }
    this.#imageTexture?.destroy();
    // Number of levels: down to the level where the longer side is 1 pixel (level L is 1/2^L of the original, rounded down)
    const mipLevelCount =
      1 + Math.floor(Math.log2(Math.max(bitmap.width, bitmap.height)));
    this.#imageTexture = this.#device.createTexture({
      size: [bitmap.width, bitmap.height],
      format: IMAGE_TEXTURE_FORMAT,
      mipLevelCount,
      // The spec requires RENDER_ATTACHMENT in addition to COPY_DST for the destination of
      // copyExternalImageToTexture (without it only the copy is discarded with a validation error,
      // and it is drawn fully transparent so the image is invisible). Also used as the target for drawing the mip chain level by level
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.COPY_DST |
        GPUTextureUsage.RENDER_ATTACHMENT,
    });
    // premultipliedAlpha loads the colors into the texture premultiplied by alpha
    // (the pipeline's blend srcFactor is 'one').
    // flipY defaults to false, so it is loaded without flipping (row 1 is row 0 of the texture).
    // uv is used as is (#writeImageQuad)
    this.#device.queue.copyExternalImageToTexture(
      { source: bitmap },
      { texture: this.#imageTexture, premultipliedAlpha: true },
      [bitmap.width, bitmap.height],
    );
    this.#buildMipLevels(mipLevelCount);
    this.#imageBindGroup = this.#device.createBindGroup({
      layout: this.#imagePipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.#viewportBuffer } },
        { binding: 1, resource: this.#imageTexture.createView() },
        { binding: 2, resource: this.#sampler },
      ],
    });
    this.#imageBitmap = bitmap;
  }

  /** Builds level 1 onward of a texture whose level 0 is loaded, drawing each at half size */
  #buildMipLevels(mipLevelCount: number): void {
    const texture = this.#imageTexture;
    if (!texture || mipLevelCount < 2) {
      return;
    }
    const encoder = this.#device.createCommandEncoder();
    let source = texture.createView({ baseMipLevel: 0, mipLevelCount: 1 });
    for (let level = 1; level < mipLevelCount; level++) {
      // Different levels of the same texture, so the read source and the write target do not overlap
      const target = texture.createView({
        baseMipLevel: level,
        mipLevelCount: 1,
      });
      const pass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: target,
            clearValue: WHITE,
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
      });
      pass.setPipeline(this.#mipPipeline);
      pass.setBindGroup(
        0,
        this.#device.createBindGroup({
          layout: this.#mipPipeline.getBindGroupLayout(0),
          entries: [
            { binding: 0, resource: source },
            { binding: 1, resource: this.#sampler },
          ],
        }),
      );
      pass.draw(3);
      pass.end();
      source = target;
    }
    this.#device.queue.submit([encoder.finish()]);
  }

  /** Turns the placement rectangle into two triangles with uv */
  #writeImageQuad(image: ImageUnderlay): GPUBuffer {
    const { x, y, width, height } = image;
    if (!this.#imageVertexBuffer) {
      this.#imageVertexBuffer = this.#device.createBuffer({
        size:
          IMAGE_VERTEX_COUNT *
          FLOATS_PER_IMAGE_VERTEX *
          Float32Array.BYTES_PER_ELEMENT,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
    }
    // The top edge (canvas y) is v=0 and the bottom edge (y + height) is v=1 (same as the texture row direction)
    this.#device.queue.writeBuffer(
      this.#imageVertexBuffer,
      0,
      new Float32Array([
        x,
        y,
        0,
        0,
        x + width,
        y,
        1,
        0,
        x,
        y + height,
        0,
        1,
        x + width,
        y,
        1,
        0,
        x + width,
        y + height,
        1,
        1,
        x,
        y + height,
        0,
        1,
      ]),
    );
    return this.#imageVertexBuffer;
  }

  /**
   * Updates the committed strokes buffer to match CommittedVertices and returns the number of vertices to draw.
   * Vertices are in units of floats (4 bytes), so the writeBuffer offset (bytes) and
   * the copyBufferToBuffer size (bytes) meet the multiple-of-4 requirement
   */
  #updateCommittedVertices(committed: CommittedVertices): number {
    if (committed.kind !== 'unchanged') {
      const kept = committed.kind === 'append' ? committed.offset : 0;
      const floats = kept + committed.data.length;
      this.#reserveCommittedVertices(
        floats * Float32Array.BYTES_PER_ELEMENT,
        kept * Float32Array.BYTES_PER_ELEMENT,
      );
      if (this.#committedVertexBuffer && committed.data.length > 0) {
        this.#device.queue.writeBuffer(
          this.#committedVertexBuffer,
          kept * Float32Array.BYTES_PER_ELEMENT,
          committed.data,
        );
      }
      this.#committedVertexFloats = floats;
    }
    return this.#committedVertexFloats / FLOATS_PER_VERTEX;
  }

  /**
   * Makes the committed strokes buffer at least bytes long. If it is too small, recreates it doubling the size,
   * and copies the first keptBytes (kept for append) to the new buffer on the GPU
   * (without holding all vertices on the CPU again)
   */
  #reserveCommittedVertices(bytes: number, keptBytes: number): void {
    const previous = this.#committedVertexBuffer;
    if (bytes === 0 || (previous && previous.size >= bytes)) {
      return;
    }
    let size = previous?.size ?? MIN_VERTEX_BUFFER_BYTES;
    while (size < bytes) {
      size *= 2;
    }
    // COPY_SRC, since it is the copy source the next time the buffer grows
    const next = this.#device.createBuffer({
      size,
      usage:
        GPUBufferUsage.VERTEX |
        GPUBufferUsage.COPY_DST |
        GPUBufferUsage.COPY_SRC,
    });
    if (previous && keptBytes > 0) {
      const encoder = this.#device.createCommandEncoder();
      encoder.copyBufferToBuffer(previous, 0, next, 0, keptBytes);
      this.#device.queue.submit([encoder.finish()]);
    }
    // The copy command has been submitted, so destroy is safe. Per the WebGPU spec (W3C "WebGPU",
    // note in 5.1 Buffer Destruction), the GPU memory of a destroyed buffer is freed
    // "after all previously submitted operations using it complete"
    previous?.destroy();
    this.#committedVertexBuffer = next;
  }

  /** Writes to the active stroke buffer from the start. If it is too small, recreates it doubling the size */
  #writeActiveVertices(vertices: Float32Array): GPUBuffer {
    if (
      !this.#activeVertexBuffer ||
      this.#activeVertexBuffer.size < vertices.byteLength
    ) {
      let size = this.#activeVertexBuffer?.size ?? MIN_VERTEX_BUFFER_BYTES;
      while (size < vertices.byteLength) {
        size *= 2;
      }

      this.#activeVertexBuffer?.destroy();
      this.#activeVertexBuffer = this.#device.createBuffer({
        size,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
    }
    this.#device.queue.writeBuffer(this.#activeVertexBuffer, 0, vertices);
    return this.#activeVertexBuffer;
  }
}
