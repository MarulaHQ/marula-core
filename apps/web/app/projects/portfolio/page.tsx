// /projects/portfolio — the quarter view route.
"use client";

import { Suspense } from "react";

import { PortfolioQuarter } from "../../../components/projects/portfolio-quarter";

export default function ProjectPortfolioPage() {
  // The quarter view reads its period and client scope from the query string,
  // which bails the page out of prerendering; Next requires the Suspense
  // boundary for that at build time.
  return (
    <Suspense fallback={null}>
      <PortfolioQuarter />
    </Suspense>
  );
}
