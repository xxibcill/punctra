export const CAMERA_PROJECTION_POLICIES = Object.freeze({
  perspective: Object.freeze({
    extentProperty: "verticalFieldOfViewRadians",
    rawMethod: "setPerspectiveCamera",
  }),
  orthographic: Object.freeze({
    extentProperty: "verticalWorldHeight",
    rawMethod: "setOrthographicCamera",
  }),
});
