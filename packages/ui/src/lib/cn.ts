/**
 * Class-name composition helper shared by all components.
 * Purpose: merge conditional class lists and de-duplicate Tailwind-style
 * conflicts. Non-purpose: no styling logic of its own.
 */
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merge class values, later values winning on conflicts. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
