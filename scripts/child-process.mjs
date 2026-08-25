export function captureChildExit(child) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      resolve({ code, signal });
    };
    child.once("error", onError);
    child.once("exit", onExit);
  });
}
