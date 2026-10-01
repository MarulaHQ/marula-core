"use client";
/**
 * Input and Textarea — reimplemented (wave41 lane 20) over the handoff kit's
 * text controls, so both are drawn the way the kit defines them (density
 * height, the kit's field border, and the aria-invalid treatment) instead of
 * the local `.m-input` / `.m-textarea` styles.
 *
 * The props are the native ones, unchanged, so call sites keep working.
 */
import { forwardRef, type InputHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { Input as KitInput, Textarea as KitTextarea } from "../kit";

// The kit's Input owns `size` (control height), so the native numeric size is omitted.
export type InputProps = Omit<InputHTMLAttributes<HTMLInputElement>, "size">;

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ className, ...props }, ref) => <KitInput ref={ref} className={className} {...props} />,
);

Input.displayName = "Input";

export type TextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement>;

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => <KitTextarea ref={ref} className={className} {...props} />,
);

Textarea.displayName = "Textarea";
