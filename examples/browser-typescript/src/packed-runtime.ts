export interface PackedRuntimeProof {
  readonly schema: "punctra-browser-packed-runtime-v1";
  readonly build: "production";
  readonly serverContract: "punctra-strict-range-v1";
  readonly viewerPackage: "@punctra/viewer";
  readonly viewerVersion: string;
  readonly viewerArtifactSha256: string;
}

export function parsePackedRuntimeProof(value: unknown): PackedRuntimeProof {
  if (typeof value !== "object" || value === null) {
    throw new Error("The packed runtime proof is not an object.");
  }
  const proof = value as Record<string, unknown>;
  if (proof.schema !== "punctra-browser-packed-runtime-v1") {
    throw new Error("The packed runtime proof schema is unsupported.");
  }
  if (proof.build !== "production") {
    throw new Error("The quickstart acceptance requires a production build.");
  }
  if (proof.serverContract !== "punctra-strict-range-v1") {
    throw new Error("The quickstart acceptance requires the strict Range server.");
  }
  if (proof.viewerPackage !== "@punctra/viewer") {
    throw new Error("The packed runtime proof names an unexpected viewer package.");
  }
  if (typeof proof.viewerVersion !== "string" || proof.viewerVersion.length === 0) {
    throw new Error("The packed runtime proof is missing the viewer version.");
  }
  if (
    typeof proof.viewerArtifactSha256 !== "string"
    || !/^[0-9a-f]{64}$/.test(proof.viewerArtifactSha256)
  ) {
    throw new Error("The packed runtime proof has an invalid viewer artifact digest.");
  }
  return Object.freeze({
    schema: proof.schema,
    build: proof.build,
    serverContract: proof.serverContract,
    viewerPackage: proof.viewerPackage,
    viewerVersion: proof.viewerVersion,
    viewerArtifactSha256: proof.viewerArtifactSha256,
  }) as PackedRuntimeProof;
}
