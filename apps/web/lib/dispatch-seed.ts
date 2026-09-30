// Frozen dispatch demo dataset — Wave 41 lane 6.
//
// Mirrors the pack's seed 1:1 (DESIGN-NOTES.md §D + source/dispatch.html) so
// lane 15 can diff captures: clock 2026-09-29T11:41, timer 00:25:05, twelve
// technicians, the same jobs, queue and absence. This is the demo organisation
// fallback only when the live API has no dispatch day for today; every value
// here is sample data.

import type {
  BoardAbsence,
  BoardJob,
  BoardTech,
  JobMode,
  Pri,
  QueueTicket,
} from "./dispatch-model";

export const ZONES: Record<string, number> = {
  "Acme Corporation": 0,
  "Ridgeline Dental": 0,
  "Northwind Legal": 1,
  "Bluebird Bakery": 1,
  "Lumen Architects": 1,
  "Oakview Vet Clinic": 2,
  "Fernhill Primary": 2,
  "Harbour Dental": 3,
  "Summit Physio": 3,
  "Kestrel Logistics": 4,
};

export const SHORT: Record<string, string> = Object.fromEntries(
  Object.keys(ZONES).map((k) => [k, (k.split(" ")[0] ?? k)]),
);

export const TECHS: BoardTech[] = [
  { id: "TL", name: "Tumi L.", role: "Service desk", sk: ["Network", "Service desk"] },
  { id: "AM", name: "Alex M.", role: "Field", sk: ["Network", "Cabling"] },
  { id: "AO", name: "Amara O.", role: "Field", sk: ["Hardware", "Network"] },
  { id: "RN", name: "Riya N.", role: "Service desk", sk: ["Security", "M365"] },
  { id: "NP", name: "Naledi P.", role: "Service desk", sk: ["M365", "Service desk"] },
  { id: "SK", name: "Sam K.", role: "Field", sk: ["Hardware", "Servers"] },
  { id: "JT", name: "Jordan T.", role: "Field", sk: ["Cabling", "Wi-Fi"] },
  { id: "KB", name: "Kagiso B.", role: "Field", sk: ["Telecoms", "Hardware"] },
  { id: "DM", name: "Dee M.", role: "Projects", sk: ["Servers", "Backup"] },
  { id: "LZ", name: "Lindiwe Z.", role: "Field", sk: ["Cabling", "Wi-Fi"] },
  { id: "PV", name: "Pieter V.", role: "Service desk", sk: ["Service desk", "Security"] },
  { id: "BH", name: "Ben H.", role: "Projects", sk: ["Servers", "Network"] },
];

type RawJob = [number, string, string, string, string, number, JobMode, Pri];

const RAW: RawJob[] = [
  [18366, "Disable leaver accounts", "Oakview Vet Clinic", "TL", "08:15", 45, "remote", "P4"],
  [18432, "VPN disconnects every 20–30 minutes", "Acme Corporation", "TL", "10:30", 120, "remote", "P1"],
  [18421, "Outlook keeps asking for a password", "Acme Corporation", "TL", "13:30", 45, "remote", "P3"],
  [18374, "Slow file server since Monday", "Acme Corporation", "TL", "14:30", 45, "remote", "P2"],
  [18380, "Replace failed UPS battery", "Ridgeline Dental", "TL", "16:00", 60, "site", "P3"],
  [18351, "Access point install", "Lumen Architects", "AM", "08:00", 90, "site", "P3"],
  [18348, "Replace core switch", "Kestrel Logistics", "AM", "10:15", 105, "site", "P2"],
  [18344, "Cabling audit", "Fernhill Primary", "AM", "13:00", 90, "site", "P3"],
  [18340, "Printer install and driver rollout", "Harbour Dental", "AM", "15:15", 75, "site", "P3"],
  [18357, "Router swap", "Summit Physio", "SK", "08:00", 90, "site", "P2"],
  [18353, "Firewall firmware update", "Bluebird Bakery", "SK", "10:45", 90, "site", "P2"],
  [18338, "Server room rewire", "Northwind Legal", "SK", "13:00", 120, "site", "P2"],
  [18333, "Office move: workstation setup", "Fernhill Primary", "SK", "15:30", 135, "site", "P3"],
  [18360, "Phishing awareness follow-up", "Northwind Legal", "RN", "09:00", 60, "remote", "P3"],
  [18327, "Conditional access review", "Summit Physio", "RN", "12:30", 60, "remote", "P2"],
  [18320, "M365 tenant migration prep", "Summit Physio", "RN", "14:00", 120, "remote", "P3"],
  [18342, "Network survey", "Fernhill Primary", "JT", "08:00", 120, "site", "P3"],
  [18339, "Wi-Fi heatmap", "Lumen Architects", "JT", "10:30", 120, "site", "P3"],
  [18331, "3CX phone install", "Bluebird Bakery", "KB", "09:00", 90, "site", "P3"],
  [18329, "Phone port and handset fix", "Oakview Vet Clinic", "KB", "11:00", 90, "site", "P2"],
  [18326, "Handset rollout", "Acme Corporation", "KB", "13:30", 120, "site", "P3"],
  [18322, "Voicemail routing fix", "Ridgeline Dental", "KB", "16:00", 60, "remote", "P3"],
  [18310, "Server migration cutover", "Northwind Legal", "DM", "08:30", 180, "site", "P2"],
  [18305, "Backup verification", "Harbour Dental", "DM", "12:30", 60, "remote", "P3"],
  [18300, "Storage upgrade", "Summit Physio", "DM", "14:15", 165, "site", "P3"],
  [18355, "MFA onboarding", "Acme Corporation", "NP", "08:00", 90, "remote", "P3"],
  [18350, "Shared calendar fix", "Lumen Architects", "NP", "10:00", 60, "remote", "P3"],
  [18346, "Licence assignment audit", "Bluebird Bakery", "NP", "13:00", 60, "remote", "P3"],
  [18316, "Structured cabling, floor 2", "Lumen Architects", "LZ", "08:30", 150, "site", "P3"],
  [18312, "Patch panel labelling", "Northwind Legal", "LZ", "13:00", 90, "site", "P4"],
  [18318, "Password policy review", "Kestrel Logistics", "PV", "09:30", 60, "remote", "P3"],
  [18308, "Security awareness session prep", "Fernhill Primary", "PV", "14:00", 90, "remote", "P3"],
  [18314, "Laptop imaging", "Harbour Dental", "AO", "09:00", 120, "site", "P3"],
  [18304, "Screen replacement", "Oakview Vet Clinic", "AO", "13:00", 60, "site", "P4"],
  [18296, "Docking stations", "Acme Corporation", "AO", "15:00", 90, "site", "P3"],
  [18302, "Firewall rule cleanup", "Kestrel Logistics", "BH", "10:00", 120, "remote", "P2"],
  [18298, "VLAN redesign", "Northwind Legal", "BH", "14:00", 120, "remote", "P3"],
];

const tm = (s: string): number => {
  const [h, m] = s.split(":");
  return Number(h) * 60 + Number(m);
};

export const seedJobs = (): BoardJob[] =>
  RAW.map(([tk, title, client, tech, s, d, mode, pri], i) => ({
    jid: `j${i + 1}`,
    tk,
    title,
    client,
    clientZone: ZONES[client] ?? 1,
    tech,
    s: tm(s),
    d,
    mode,
    pri,
  }));

/** The pack's state 07 day: Alex M.'s core switch overruns into the cabling
 *  audit, so the banner has something real to say. The default day is
 *  collision-free on purpose — an unapproved double booking has to come from
 *  somewhere (a job booked straight onto the calendar), not from the planner.
 *  `?demo=collision` on /dispatch loads this one. */
export const seedWithCollision = (): BoardJob[] =>
  seedJobs().map((j) => (j.tk === 18348 ? { ...j, d: 180 } : j));

export const seedAbsences = (): BoardAbsence[] => [
  { jid: "a1", tech: "JT", s: tm("14:00"), d: 240, title: "Off: appointment" },
];

export const seedQueue = (): QueueTicket[] => [
  { tk: 18441, title: "Phishing email reported by reception", client: "Harbour Dental", clientZone: 3, pri: "P2", est: 30, mode: "remote", need: "Security", sla: 50 },
  { tk: 18439, title: "Adobe licence expired for two designers", client: "Lumen Architects", clientZone: 1, pri: "P3", est: 30, mode: "remote", need: "M365", sla: 400 },
  { tk: 18437, title: "Firewall reports high CPU", client: "Kestrel Logistics", clientZone: 4, pri: "P2", est: 60, mode: "remote", need: "Network", sla: 95 },
  { tk: 18435, title: "Add new user to Finance share", client: "Bluebird Bakery", clientZone: 1, pri: "P4", est: 30, mode: "remote", need: "M365", sla: null },
  { tk: 18433, title: "Meeting room display will not connect", client: "Fernhill Primary", clientZone: 2, pri: "P3", est: 60, mode: "site", need: "Hardware", sla: 480 },
  { tk: 18430, title: "Domain renewal due in 21 days", client: "Fernhill Primary", clientZone: 2, pri: "P4", est: 15, mode: "remote", need: "Service desk", sla: null },
];

/** The dispatcher looking at this board. The pack marks the current user's lane
 *  with an ink avatar instead of the team green, which is the only way to find
 *  your own row in twelve lanes. */
export const ME_TECH = "TL";

export const FROZEN_CLOCK = "11:41";
export const FROZEN_DATE_LINE = "Tue 29 Sep 2026";
export const FROZEN_TIMER = "00:25:05";
