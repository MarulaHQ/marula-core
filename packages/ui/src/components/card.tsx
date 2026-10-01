"use client";
/**
 * Card — reimplemented (wave41 lane 20) over the handoff kit's `Card`.
 *
 * The old `.m-card` was a 3px-radius surface, which was the pre-Marula rule;
 * the mockups draw cards with the kit's 16px radius and a 1px border. `Card`,
 * `CardHeader` and `CardContent` now render the kit's pieces so every surface
 * gets the canonical container. The names and call signatures are kept, and
 * they now also accept className and the rest of the div props.
 */
import type { HTMLAttributes, ReactNode } from "react";
import { Card as KitCard, CardBody, CardHeader as KitCardHeader } from "../kit";

export type CardProps = HTMLAttributes<HTMLDivElement> & { children?: ReactNode };

export function Card({ children, ...props }: CardProps) {
  return <KitCard {...props}>{children}</KitCard>;
}

export function CardHeader({ children, ...props }: CardProps) {
  return <KitCardHeader {...props}>{children}</KitCardHeader>;
}

export function CardContent({ children, ...props }: CardProps) {
  return <CardBody {...props}>{children}</CardBody>;
}
