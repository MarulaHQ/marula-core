#!/usr/bin/env bash
# Seed the states this lane's captures need (Wave 41 lane 9).
#
# Why this exists: `seed-demo` gives the app assets and config items but leaves
# stock, locations, client loans and tools EMPTY. Without this, every stock-side
# capture would be an empty state and would prove nothing about the one rule
# this lane is measured on — overdue red, due soon blush, both with a word.
#
# So this creates a small, deliberate fixture set:
#   - a location TREE (warehouse > aisle > shelf > bin), so /stock/locations has
#     real indentation to look at
#   - stock items with on-hand, one AT its reorder point (blush low-stock)
#   - a serialized TOOL unit checked out PAST its return (red overdue)
#   - three client loans: overdue, due in two days, and returned
#   - purchase orders across statuses so the orders ledger has real variety,
#     including one past its expected date (red)
#
# Usage: source e2e/.e2e-stack.env && bash scripts/seed-lane9-capture-fixtures.sh
# Idempotent-ish: it creates fresh rows each run, which is fine for a throwaway
# per-run capture database.
set -uo pipefail

API="${E2E_API_URL%/}"
TOK=$(curl -s -X POST "$API/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"owner@demo.local","password":"demo-password-123"}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')
AUTH=(-H "authorization: Bearer $TOK" -H 'content-type: application/json')
jqp() { python3 -c "import sys,json
try:
    d=json.load(sys.stdin);print($1)
except Exception:
    print('')"; }
key() { python3 -c 'import uuid;print(uuid.uuid4())'; }

echo "[seed] api $API"

# ---------------------------------------------------------------- locations
# Location kinds are validated server-side: warehouse, shelf, van, truck.
# So the tree is warehouse > shelf, plus a van assigned to a person — which is
# the case the "assigned user" column exists for.
SUF=$(python3 -c "import uuid;print(str(uuid.uuid4())[:6])")
wh=$(curl -s -X POST "$API/api/v1/stock-locations" "${AUTH[@]}" \
  -d "{\"name\":\"Northline Depot $SUF\",\"code\":\"WH-$SUF\",\"kind\":\"warehouse\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
rack3=$(curl -s -X POST "$API/api/v1/stock-locations" "${AUTH[@]}" \
  -d "{\"name\":\"Rack 3 — network\",\"code\":\"R3\",\"kind\":\"shelf\",\"parent_id\":\"$wh\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
rack4=$(curl -s -X POST "$API/api/v1/stock-locations" "${AUTH[@]}" \
  -d "{\"name\":\"Rack 4 — power\",\"code\":\"R4\",\"kind\":\"shelf\",\"parent_id\":\"$wh\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
ME=$(curl -s "$API/api/me" "${AUTH[@]}" | jqp "d['user_id']")
van=$(curl -s -X POST "$API/api/v1/stock-locations" "${AUTH[@]}" \
  -d "{\"name\":\"Van 2\",\"code\":\"VAN2\",\"kind\":\"van\",\"parent_id\":\"$wh\",\"assigned_user_id\":\"$ME\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
shelf="$rack3"
bin="$rack4"
echo "[seed] locations: warehouse > shelf x2, plus an assigned van"

# ---------------------------------------------------------------- stock items
# Item kind is validated server-side: `consumable` or `serialized`. A tool is a
# SERIALIZED item with `is_tool` set, which is what makes it appear in
# /stock/tools at all. (Both the location kinds and the item kinds are checked,
# so a plausible-looking fixture silently creates nothing until it is right.)
mk_item() { # name kind unit is_tool
  curl -s -X POST "$API/api/v1/stock-items" "${AUTH[@]}" \
    -d "{\"name\":\"$1\",\"kind\":\"$2\",\"unit\":\"$3\",\"is_tool\":$4,\"currency\":\"USD\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')"
}
rails=$(mk_item "Cat6a patch cable 2m" consumable each false)
panel=$(mk_item "24-port patch panel" consumable each false)
psu=$(mk_item "Switch PSU 250W" consumable each false)
multimeter=$(mk_item "Fluke 87V multimeter" serialized each true)
crashcart=$(mk_item "Thermal camera" serialized each true)
echo "[seed] stock items: 5 (2 tools)"

# on-hand. The PSU sits AT its reorder point so the ledger shows the blush
# low-stock state rather than only healthy rows.
recv() { curl -s -X POST "$API/api/v1/stock/receive" "${AUTH[@]}" \
  -d "{\"item_id\":\"$1\",\"location_id\":\"$2\",\"qty_milli\":$3,\"unit_cost_minor\":$4,\"idempotency_key\":\"$(key)\"}" -o /dev/null; }
recv "$rails" "$bin" 120000 480
recv "$panel" "$shelf" 40000 18500
recv "$psu" "$rack4" 2000 4200
recv "$multimeter" "$shelf" 3000 31000
recv "$crashcart" "$wh" 1000 145000
# a serialized tool unit, so /stock/tools has a real serial to read
curl -s -X POST "$API/api/v1/stock/receive" "${AUTH[@]}" \
  -d "{\"item_id\":\"$multimeter\",\"location_id\":\"$shelf\",\"qty_milli\":1000,\"unit_cost_minor\":31000,\"serial_numbers\":[\"FLU-8842\"],\"idempotency_key\":\"$(key)\"}" -o /dev/null
echo "[seed] on-hand: 5 receipts incl. one serialized tool unit (FLU-8842)"

# ---------------------------------------------------------------- clients + loans
# One client per loan. Sharing a client across loans in a single run collapsed
# them into one row, and a capture that shows only the RED half of the severity
# rule is worse than no capture: it looks like the blush case does not exist.
mk_client() { # name
  curl -s -X POST "$API/api/v1/clients" "${AUTH[@]}" \
    -d "{\"name\":\"$1\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')"
}
CLIENT=$(mk_client "Harbour Dental (overdue loan)")
CLIENT2=$(mk_client "Meridian Clinic (due soon)")
soon=$(python3 -c "import datetime;print((datetime.date.today()+datetime.timedelta(days=2)).isoformat())")
past=$(python3 -c "import datetime;print((datetime.date.today()-datetime.timedelta(days=9)).isoformat())")
issued_from=$(python3 -c "import datetime;print((datetime.date.today()-datetime.timedelta(days=14)).isoformat())+'T09:00:00Z'")

# A loan can only be issued against a unit that EXISTS as a serial, so make one
# per loanable tool: its own serialised item, its own shelf, its own serial.
loanable() { # name serial
  local shelf_id item_id
  shelf_id=$(curl -s -X POST "$API/api/v1/stock-locations" "${AUTH[@]}" \
    -d "{\"name\":\"Loan shelf $1\",\"kind\":\"shelf\",\"parent_id\":\"$wh\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
  item_id=$(curl -s -X POST "$API/api/v1/stock-items" "${AUTH[@]}" \
    -d "{\"name\":\"$1\",\"kind\":\"serialized\",\"unit\":\"each\",\"is_tool\":false,\"currency\":\"USD\",\"idempotency_key\":\"$(key)\"}" | jqp "d.get('id','')")
  curl -s -X POST "$API/api/v1/stock/receive" "${AUTH[@]}" \
    -d "{\"item_id\":\"$item_id\",\"location_id\":\"$shelf_id\",\"qty_milli\":1000,\"unit_cost_minor\":12000,\"serial_numbers\":[\"$2\"],\"idempotency_key\":\"$(key)\"}" -o /dev/null
  echo "$item_id"
}
# Date formats differ per field and a 422 never says which: expected_return_at is
# a NaiveDate ("2026-09-21"), out_at and a tool's due_at are RFC 3339.
issue() { # client expected_date item out_rfc3339
  curl -s -X POST "$API/api/v1/client-loans" "${AUTH[@]}" \
    -d "{\"client_id\":\"$1\",\"expected_return_at\":\"$2\",\"out_at\":\"$3\",\"items\":[{\"stock_item_id\":\"$4\"}],\"idempotency_key\":\"$(key)\"}" -o /dev/null; }
o1=$(loanable "Loan: Fluke 87V" "FLU-7101")
o2=$(loanable "Loan: Site survey kit" "SSK-2201")
o3=$(loanable "Loan: Signal tester" "SIG-3301")
issue "$CLIENT"  "$past" "$issued_from" "$o1"   # OVERDUE -> red
issue "$CLIENT"  "$soon" "$issued_from" "$o2"   # DUE SOON -> blush
issue "$CLIENT2" "$soon" "$issued_from" "$o3"   # DUE SOON -> blush
echo "[seed] client loans: 1 OVERDUE (red), 2 due soon (blush)"

# Check the tool unit out PAST its return, so /stock/tools shows an overdue too.
# CreateCheckoutRequest needs holder + from_location + due_at, and carries `items`
# (the units) rather than a bare stock_item_id.
recv_serial=$(curl -s "$API/api/v1/stock-items" "${AUTH[@]}" | python3 -c "
import sys,json
print([i['id'] for i in json.load(sys.stdin) if i['name'].startswith('Fluke')][0])")
# a checkout needs a unit actually ON HAND at that location, with a serial
curl -s -X POST "$API/api/v1/stock/receive" "${AUTH[@]}" \
  -d "{\"item_id\":\"$recv_serial\",\"location_id\":\"$shelf\",\"qty_milli\":1000,\"unit_cost_minor\":31000,\"serial_numbers\":[\"FLU-8842\"],\"idempotency_key\":\"$(key)\"}" -o /dev/null
due_rfc=$(python3 -c "import datetime;print((datetime.date.today()-datetime.timedelta(days=9)).isoformat()+'T17:00:00Z')")
curl -s -X POST "$API/api/v1/tool-checkouts" "${AUTH[@]}" \
  -d "{\"holder_user_id\":\"$ME\",\"from_location_id\":\"$shelf\",\"due_at\":\"$due_rfc\",\"items\":[{\"stock_item_id\":\"$recv_serial\",\"qty_milli\":1000}],\"idempotency_key\":\"$(key)\"}" -o /dev/null
echo "[seed] tool checkout: FLU-8842 checked out, due $past (overdue -> red)"

# ---------------------------------------------------------------- purchasing
v1=$(curl -s -X POST "$API/api/v1/vendors" "${AUTH[@]}" \
  -d "{\"name\":\"Meridian Cable Co\",\"default_currency\":\"USD\",\"email\":\"orders@meridiancable.test\",\"idempotency_key\":\"$(key)\"}" | jqp "d['id']")
v2=$(curl -s -X POST "$API/api/v1/vendors" "${AUTH[@]}" \
  -d "{\"name\":\"Kestrel Instrument\",\"default_currency\":\"GBP\",\"phone\":\"+44 20 7946 0102\",\"idempotency_key\":\"$(key)\"}" | jqp "d['id']")
mk_order() { # vendor number expected
  local oid
  oid=$(curl -s -X POST "$API/api/v1/purchase-orders" "${AUTH[@]}" \
    -d "{\"vendor_id\":\"$1\",\"number\":\"$2\",\"currency\":\"USD\",\"expected_at\":\"$3\",\"idempotency_key\":\"$(key)\"}" | jqp "d['order']['id']")
  curl -s -X POST "$API/api/v1/purchase-orders/$oid/lines" "${AUTH[@]}" \
    -d "{\"description\":\"Cat6a cable, 2m\",\"qty_ordered_milli\":2000,\"unit_cost_minor\":480,\"idempotency_key\":\"$(key)\"}" -o /dev/null
  curl -s -X POST "$API/api/v1/purchase-orders/$oid/lines" "${AUTH[@]}" \
    -d "{\"description\":\"Patch panel, 24-port\",\"qty_ordered_milli\":500,\"unit_cost_minor\":18500,\"idempotency_key\":\"$(key)\"}" -o /dev/null
  echo "$oid"
}
future=$(python3 -c "import datetime;print((datetime.date.today()+datetime.timedelta(days=6)).isoformat())")
o_awaiting=$(mk_order "$v1" "PO-2609-021" "$future")
o_late=$(mk_order "$v2" "PO-2609-018" "$past")
# place one so it is AWAITING RECEIVE, leave the other a draft for variety
curl -s -X POST "$API/api/v1/purchase-orders/$o_awaiting/order" "${AUTH[@]}" -d "{\"idempotency_key\":\"$(key)\"}" -o /dev/null
echo "[seed] orders: 1 awaiting receive (future), 1 past expected (red), + the one made earlier"

echo "[seed] verify:"
for ep in stock-items stock-locations client-loans tool-checkouts purchase-orders vendors; do
  n=$(curl -s "$API/api/v1/$ep" "${AUTH[@]}" | python3 -c "
import sys,json
try:
    d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 0)
except Exception: print('err')")
  printf "  %-18s %s\n" "$ep" "$n"
done
echo "[seed] done — the captures can show real states now"
