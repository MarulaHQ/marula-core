"use client";
/**
 * RackElevation — a 42U rack drawn as units, one device per placement.
 *
 * Purpose: the pack's K05 anatomy. A rack is a physical fact, so it is drawn
 * rather than described: unit numbers on the left, devices placed at their
 * real U position and height, front and rear as separate views, and a legend
 * that names every device type because colour alone is never the signal.
 *
 * Every rack also renders as a table (visually hidden, reachable by a screen
 * reader): an SVG-or-CSS drawing is not a table, and the COMPONENT-ROADMAP
 * requires the table alternative.
 *
 * Non-purpose: no drag-and-drop and no editing. Placement is a form action on
 * the record; this component renders what the server stored.
 */

import type { RackDevice } from "./knowledge-api";

const CELL = 18; // px per U; 12px text fits a 1U device at this height.

const TYPE_STYLE: Record<
  RackDevice["device_type"],
  { background: string; border: string; color: string; dashed?: boolean }
> = {
  network: {
    background: "var(--m-surface-2)",
    border: "1px solid var(--m-border)",
    color: "var(--m-text)",
  },
  compute: {
    background: "var(--m-surface)",
    border: "1.5px solid var(--m-text)",
    color: "var(--m-text)",
  },
  storage: {
    background: "var(--m-accent-soft)",
    border: "1px solid var(--m-accent)",
    color: "var(--m-on-accent)",
  },
  power: {
    background: "var(--m-status-due-bg)",
    border: "1px solid var(--m-status-due-fg)",
    color: "var(--m-status-due-fg)",
  },
  patch: {
    background: "var(--m-surface)",
    border: "1px dashed var(--m-border)",
    color: "var(--m-text-secondary)",
    dashed: true,
  },
  other: {
    background: "var(--m-surface-2)",
    border: "1px solid var(--m-border)",
    color: "var(--m-text-secondary)",
  },
};

const TYPE_LABEL: Record<RackDevice["device_type"], string> = {
  network: "Network",
  compute: "Compute",
  storage: "Storage",
  power: "Power",
  patch: "Passive",
  other: "Other",
};

/** The CSS top of a device whose bottom unit is `unit_start` and which is
 * `unit_height` units tall, measured from the top of the rack. */
export function deviceOffset(units: number, unitStart: number, unitHeight: number): number {
  return (units - unitStart - unitHeight + 1) * CELL;
}

export function RackElevation({
  units,
  devices,
  face = "front",
  label,
}: {
  units: number;
  devices: RackDevice[];
  face?: "front" | "rear";
  label: string;
}): React.JSX.Element {
  const onFace = devices.filter((d) => d.face === face);
  // Occupied units as a union, not a sum: two devices that overlap (a race the
  // writes do not prevent) must not claim more units than the rack has.
  const occupied = new Set<number>();
  for (const device of onFace) {
    for (let unit = device.unit_start; unit < device.unit_start + device.unit_height; unit += 1) {
      if (unit >= 1 && unit <= units) occupied.add(unit);
    }
  }
  const used = occupied.size;

  return (
    <div className="flex flex-col gap-3">
      <div
        className="relative rounded-[8px]"
        style={{
          width: 292,
          height: units * CELL + 4,
          border: "2px solid var(--m-border)",
          background: "var(--m-surface-2)",
        }}
        role="img"
        aria-label={`${label}, ${face} view. ${onFace.length} devices, ${used} of ${units} units used.`}
      >
        {Array.from({ length: Math.floor(units / 6) }, (_, i) => {
          const unit = units - i * 6;
          return (
            <span
              key={unit}
              aria-hidden="true"
              className="absolute text-[12px] tabular-nums"
              style={{
                left: 0,
                width: 26,
                textAlign: "right",
                top: (units - unit) * CELL + 3,
                color: "var(--m-text-muted)",
              }}
            >
              {unit}
            </span>
          );
        })}
        {onFace.map((device) => {
          const style = TYPE_STYLE[device.device_type];
          const statusWord =
            device.status === "attention"
              ? "Needs attention"
              : device.status === "offline"
                ? "Offline"
                : device.status === "passive"
                  ? "Passive"
                  : "Online";
          // A placement that runs past the top of the rack would draw at a
          // negative offset; keep the box inside the frame instead.
          const top = Math.min(
            Math.max(0, deviceOffset(units, device.unit_start, device.unit_height) + 2),
            Math.max(0, units * CELL - CELL),
          );
          const height = Math.max(
            CELL - 3,
            Math.min(device.unit_height * CELL - 3, units * CELL - top),
          );
          return (
            <div
              key={device.id}
              className="absolute flex items-center gap-2 overflow-hidden rounded-[4px] px-2 text-[12px] font-semibold"
              style={{
                left: 30,
                right: 6,
                top,
                height,
                background: style.background,
                border: style.border,
                color: style.color,
              }}
              title={`${device.name} · ${TYPE_LABEL[device.device_type]} · U${device.unit_start} · ${statusWord}`}
            >
              <span className="truncate">{device.name}</span>
              {device.status === "attention" ? (
                <span
                  aria-hidden="true"
                  className="ml-auto inline-block h-2 w-2 shrink-0 rounded-full"
                  style={{ background: "var(--m-blush)" }}
                />
              ) : null}
            </div>
          );
        })}
      </div>

      <div className="flex flex-wrap gap-1.5" aria-hidden="true">
        {(Object.keys(TYPE_LABEL) as Array<RackDevice["device_type"]>).map((type) => {
          const count = onFace.filter((d) => d.device_type === type).length;
          if (count === 0) return null;
          const style = TYPE_STYLE[type];
          return (
            <span
              key={type}
              className="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[12px] font-semibold"
              style={{
                background: style.background,
                border: style.border,
                color: style.color,
              }}
            >
              {TYPE_LABEL[type]}
              <span className="tabular-nums">{count}</span>
            </span>
          );
        })}
      </div>

      <table className="sr-only">
        <caption>{`${label}, ${face} view`}</caption>
        <thead>
          <tr>
            <th scope="col">Unit</th>
            <th scope="col">Device</th>
            <th scope="col">Type</th>
            <th scope="col">Status</th>
          </tr>
        </thead>
        <tbody>
          {onFace.map((device) => (
            <tr key={device.id}>
              <td>
                {device.unit_height > 1
                  ? `U${device.unit_start} to U${device.unit_start + device.unit_height - 1}`
                  : `U${device.unit_start}`}
              </td>
              <td>{device.name}</td>
              <td>{TYPE_LABEL[device.device_type]}</td>
              <td>{device.status}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
