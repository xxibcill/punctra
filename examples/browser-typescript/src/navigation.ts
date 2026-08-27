import type { ViewerCamera, ViewerState } from "@punctra/viewer";
import type { NormalizedViewerInput } from "@punctra/viewer/input";

type Vector3 = readonly [number, number, number];

export function cameraFromState(state: ViewerState): ViewerCamera {
  const camera = state.camera;
  const shared = {
    eye: vector3(camera.eye),
    target: vector3(camera.target),
    up: vector3(camera.up),
    nearDistance: camera.nearDistance,
    farDistance: camera.farDistance,
  };
  return camera.projection === "perspective"
    ? { ...shared, projection: "perspective", verticalFieldOfViewRadians: camera.verticalFieldOfViewRadians }
    : { ...shared, projection: "orthographic", verticalWorldHeight: camera.verticalWorldHeight };
}

export function applyNavigation(
  camera: ViewerCamera,
  input: NormalizedViewerInput,
  viewportHeight: number,
): ViewerCamera | null {
  if ("deltaX" in input) {
    return input.kind === "orbit"
      ? orbitCamera(camera, input.deltaX, input.deltaY)
      : panCamera(camera, input.deltaX, input.deltaY, viewportHeight);
  }
  if (input.kind === "zoom") return zoomCamera(camera, input.delta);
  return input.code === "KeyP" ? alternateProjection(camera) : null;
}

export function alternateProjection(camera: ViewerCamera): ViewerCamera {
  const shared = {
    eye: vector3(camera.eye),
    target: vector3(camera.target),
    up: vector3(camera.up),
    nearDistance: camera.nearDistance,
    farDistance: camera.farDistance,
  };
  if (camera.projection === "perspective") {
    const radius = length(subtract(camera.eye, camera.target));
    return {
      ...shared,
      projection: "orthographic",
      verticalWorldHeight: 2 * radius * Math.tan(camera.verticalFieldOfViewRadians / 2),
    };
  }
  return {
    ...shared,
    projection: "perspective",
    verticalFieldOfViewRadians: Math.PI / 3,
  };
}

function orbitCamera(camera: ViewerCamera, horizontalPixels: number, verticalPixels: number): ViewerCamera {
  const offset = subtract(camera.eye, camera.target);
  const radius = length(offset);
  const azimuth = Math.atan2(offset[1], offset[0]) - horizontalPixels * 0.006;
  const elevation = clamp(Math.asin(offset[2] / radius) + verticalPixels * 0.006, 0.08, 1.48);
  const horizontalRadius = radius * Math.cos(elevation);
  const eye = vector3([
    camera.target[0] + horizontalRadius * Math.cos(azimuth),
    camera.target[1] + horizontalRadius * Math.sin(azimuth),
    camera.target[2] + radius * Math.sin(elevation),
  ]);
  return { ...camera, eye };
}

function panCamera(
  camera: ViewerCamera,
  horizontalPixels: number,
  verticalPixels: number,
  viewportHeight: number,
): ViewerCamera {
  const forward = normalize(subtract(camera.target, camera.eye));
  const right = normalize(cross(forward, camera.up));
  const up = normalize(cross(right, forward));
  const verticalHeight = visibleHeight(camera);
  const scale = verticalHeight / Math.max(1, viewportHeight);
  const movement = add(
    scaleVector(right, -horizontalPixels * scale),
    scaleVector(up, verticalPixels * scale),
  );
  return {
    ...camera,
    eye: add(camera.eye, movement),
    target: add(camera.target, movement),
  };
}

function zoomCamera(camera: ViewerCamera, lines: number): ViewerCamera {
  const factor = Math.exp(lines * 0.12);
  if (camera.projection === "orthographic") {
    return { ...camera, verticalWorldHeight: Math.max(0.01, camera.verticalWorldHeight * factor) };
  }
  const offset = scaleVector(subtract(camera.eye, camera.target), factor);
  return { ...camera, eye: add(camera.target, offset) };
}

function visibleHeight(camera: ViewerCamera): number {
  if (camera.projection === "orthographic") return camera.verticalWorldHeight;
  return 2 * length(subtract(camera.eye, camera.target))
    * Math.tan(camera.verticalFieldOfViewRadians / 2);
}

function vector3(value: readonly number[]): Vector3 {
  return [value[0] ?? 0, value[1] ?? 0, value[2] ?? 0];
}

function add(left: readonly number[], right: readonly number[]): Vector3 {
  return vector3(left.map((value, axis) => value + (right[axis] ?? 0)));
}

function subtract(left: readonly number[], right: readonly number[]): Vector3 {
  return vector3(left.map((value, axis) => value - (right[axis] ?? 0)));
}

function scaleVector(vector: readonly number[], scale: number): Vector3 {
  return vector3(vector.map((value) => value * scale));
}

function length(vector: readonly number[]): number {
  return Math.hypot(...vector);
}

function normalize(vector: readonly number[]): Vector3 {
  const magnitude = length(vector);
  return scaleVector(vector, 1 / magnitude);
}

function cross(left: readonly number[], right: readonly number[]): Vector3 {
  return [
    (left[1] ?? 0) * (right[2] ?? 0) - (left[2] ?? 0) * (right[1] ?? 0),
    (left[2] ?? 0) * (right[0] ?? 0) - (left[0] ?? 0) * (right[2] ?? 0),
    (left[0] ?? 0) * (right[1] ?? 0) - (left[1] ?? 0) * (right[0] ?? 0),
  ];
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}
