import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { usePunctraViewer } from "@punctra/react";

function App() {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const [active, setActive] = useState(true);
  const binding = usePunctraViewer({
    canvas,
    active,
    viewport: {
      cssWidth: 960,
      cssHeight: 540,
      devicePixelRatio: window.devicePixelRatio,
    },
    assets: { cacheKey: "react-trial" },
  });

  return (
    <main>
      <canvas ref={setCanvas} width={960} height={540} />
      <button type="button" onClick={() => setActive((value) => !value)}>
        {active ? "Pause" : "Resume"}
      </button>
      <output>{binding.status}</output>
    </main>
  );
}

const rootElement = document.querySelector<HTMLElement>("#root");
if (!rootElement) throw new Error("React root is missing");
createRoot(rootElement).render(<StrictMode><App /></StrictMode>);
