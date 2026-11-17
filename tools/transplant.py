"""Setzt den aktiven Lkw aus einem ETS2-Spielstand in einen anderen. Siehe README.md."""
import sys, re
HDR = 'SiiNunit\n{\n'
UNIT = re.compile(r'^(\w+) : (\S+) \{$')

class Save:
    def __init__(self, path):
        self.text = open(path, encoding='utf-8').read()
        assert self.text.startswith(HDR) and self.text.endswith('\n}')
        self.units = {}   # name -> [type, lines]
        self.order = []
        cur = None
        for ln in self.text[len(HDR):-1].split('\n'):
            if cur is None:
                m = UNIT.match(ln)
                if m:
                    cur = (m.group(1), m.group(2), [])
                else:
                    assert ln == '', repr(ln)
            elif ln == '}':
                assert cur[1] not in self.units, cur[1]
                self.units[cur[1]] = [cur[0], cur[2]]
                self.order.append(cur[1]); cur = None
            else:
                cur[2].append(ln)
        assert cur is None
    def dump(self):
        out = [HDR]
        for n in self.order:
            t, l = self.units[n]
            out.append(f"{t} : {n} {{\n" + ''.join(x + '\n' for x in l) + '}\n\n')
        out.append('}')
        return ''.join(out)
    def refs(self, name):
        r = []
        for x in self.units[name][1]:
            if ': ' in x:
                v = x.split(': ', 1)[1].strip()
                if v in self.units: r.append(v)
        return r
    def closure(self, name, nameless_only=False):
        seen = []; done = set(); stack = [name]
        while stack:
            n = stack.pop()
            if n in done: continue
            done.add(n); seen.append(n)
            stack.extend(r for r in self.refs(n) if not nameless_only or r.startswith('_nameless.'))
        return seen
    def get(self, name, key):
        for x in self.units[name][1]:
            if x.startswith(f' {key}: '): return x.split(': ', 1)[1]
        raise KeyError(key)
    def set(self, name, key, val):
        l = self.units[name][1]
        for i, x in enumerate(l):
            if x.startswith(f' {key}: '):
                l[i] = f' {key}: {val}'; return
        raise KeyError(key)
    def set_array(self, name, key, vals):
        l = self.units[name][1]
        idx = [i for i, x in enumerate(l) if x.startswith(f' {key}: ') or x.startswith(f' {key}[')]
        assert idx and idx == list(range(idx[0], idx[-1] + 1)), key
        new = [f' {key}: {len(vals)}'] + [f' {key}[{i}]: {v}' for i, v in enumerate(vals)]
        l[idx[0]:idx[-1] + 1] = new
    def find_type(self, t):
        return [n for n in self.order if self.units[n][0] == t]
    def referrers(self, name):
        return [n for n in self.order if name in self.refs(n)]
    def check(self, label):
        dangling = []
        for n in self.order:
            for x in self.units[n][1]:
                if ': ' in x:
                    v = x.split(': ', 1)[1].strip()
                    if v.startswith('_nameless.') and v not in self.units: dangling.append((n, x.strip()))
        root = self.order[0]
        reach = set(self.closure(root))
        orphans = [n for n in self.order if n not in reach]
        print(f"[{label}] units={len(self.order)} dangling={len(dangling)} orphans={len(orphans)}")
        for d in dangling[:10]: print("   dangling:", d)
        for o in orphans[:10]: print("   orphan:", self.units[o][0], o)
        return not dangling, orphans

if __name__ == '__main__':
    src_path, dst_path, out_path = sys.argv[1:4]
    S = Save(src_path); T = Save(dst_path)
    assert S.dump() == S.text and T.dump() == T.text, "roundtrip"
    print("roundtrip ok")
    S.check('source original'); _, base_orph = T.check('target original')

    sp = S.find_type('player')[0]; tp = T.find_type('player')[0]
    # --- source truck: the one currently assigned
    s_assigned = S.get(sp, 'assigned_vehicles')
    V = S.get(s_assigned, 'vehicle')
    assert S.units[V][0] == 'vehicle'
    vclos = S.closure(V, True)
    print("source truck", V, "units:", len(vclos), "plate:", S.get(V, 'license_plate'))
    brand = None
    for a in vclos:
        try:
            dp = S.get(a, 'data_path')
        except KeyError: continue
        m = re.match(r'"/def/vehicle/truck/([^/]+)/data\.sii"', dp)
        if m: brand = m.group(1)
    assert brand; print("model:", brand)
    coll = [n for n in vclos if n in T.units]
    assert not coll, coll

    # --- target: remove quick job, company truck, trailer
    t_assigned = T.get(tp, 'assigned_vehicles')
    old_v = T.get(t_assigned, 'vehicle'); old_t = T.get(t_assigned, 'trailer'); job = T.get(tp, 'current_job')
    delete = []
    for root in (old_v, old_t, job):
        if root != 'null': delete += T.closure(root, True)
    delete = list(dict.fromkeys(delete))
    allowed = {tp, t_assigned}
    for n in delete:
        ext = [r for r in T.referrers(n) if r not in delete and r not in allowed]
        assert not ext, (n, ext)
    print("deleting", len(delete), "units (job, company truck, trailer)")
    for n in delete:
        del T.units[n]; T.order.remove(n)

    # --- target: insert truck
    PL = '_nameless.2ff.7a11.0008'
    assert PL not in T.units
    ins = T.order.index(tp) + 1
    new_units = [(n, S.units[n][0], list(S.units[n][1])) for n in vclos]
    new_units.append((PL, 'profit_log', [' stats_data: 0', ' acc_distance_free: 0', ' acc_distance_on_job: 0', ' history_age: nil']))
    for i, (n, t, l) in enumerate(new_units):
        T.units[n] = [t, l]; T.order.insert(ins + i, n)

    placement = T.get(t_assigned, 'stored_vehicle_placement')
    T.set(tp, 'current_job', 'null')
    T.set(tp, 'my_vehicles_mode', 'truck')
    T.set_array(tp, 'trucks', [V])
    T.set_array(tp, 'truck_profit_logs', [PL])
    T.set(t_assigned, 'vehicle', V)
    T.set(t_assigned, 'trailer', 'null')
    T.set_array(t_assigned, 'stored_trailer_placements', [])
    T.set(t_assigned, 'stored_trailer_attached', 'false')
    mine = T.get(tp, 'my_vehicles[0]')
    T.set(mine, 'vehicle', V)
    T.set(mine, 'stored_vehicle_placement', placement)
    hq = T.get(tp, 'hq_city')
    T.set(mine, 'stored_vehicle_city', f'city.{hq}')
    g = f'garage.{hq}'
    assert T.get(g, 'vehicles[0]') == 'null' and T.get(g, 'drivers[0]') == T.get(tp, 'drivers[0]')
    T.set(g, 'vehicles[0]', V)
    gp = T.find_type('game_progress')[0]
    T.set_array(gp, 'owned_trucks', [f'"vehicle.{brand}"'])

    ok, orph = T.check('target modified')
    assert ok and len(orph) == len(base_orph), "integrity"
    open(out_path, 'w', encoding='utf-8', newline='\n').write(T.dump())
    R = Save(out_path); assert R.dump() == R.text
    print("written", out_path, len(R.text), "bytes; trucks:", R.get(tp, 'trucks[0]'), "garage:", R.get(g, 'vehicles[0]'))
