"""Nachbildung des Scroll-Abgleichs aus src/ScrollCapture.cs (v1.2.0) zum Testen ohne Windows.

Simuliert Webseiten (Text, Fotos, Leerflaechen) mit Stoerungen (Rauschen, halbtransparente Kopfleiste,
Animation, feste Seitenleiste, Banner unten) und prueft, ob das zusammengesetzte Bild lueckenlos und
ohne Streifen ist. Aufruf: python3 tools/scroll_stitch_sim.py  (Laufzeit einige Minuten)
"""
# Nachbildung des toleranten Abgleichs (ScrollCapture.cs v1.1.1) mit realistischen Störungen
import random
K=32; TOL=8; MINC=6; MinOverlap=24; MinDistinct=8

EXACT=1
FALLBACKS=[0]
LAST={}
def active_mask(A,B):
    H=len(A); m=[]
    for k in range(K):
        eq=sum(1 for y in range(H) if abs(A[y][k]-B[y][k])<=EXACT)
        m.append(eq < 0.97*H)
    return [k for k in range(K) if m[k]]
def distinct(r,act): 
    v=[r[k] for k in act]; return max(v)-min(v)>=MINC
def match_old(a,b,act):
    lim=TOL*len(act); s=0
    for k in act:
        s+=abs(a[k]-b[k])
        if s>lim: return False
    return True
BINTOL=10
def row_err(a,b,act):
    ok=0; e=0
    for k in act:
        d=abs(a[k]-b[k])
        if d<=BINTOL: ok+=1; e+=d
    return ok/len(act), (e/ok if ok else 99)
def match(a,b,act): return row_err(a,b,act)[0]>=0.75
def same_row(a,b,act): return all(abs(a[k]-b[k])<=EXACT for k in act)
def nearly_same(A,B,share=0.9):
    if A==B: return True
    act=active_mask(A,B)
    if len(act)<4: return True
    n=ok=0
    for a,b in zip(A,B):
        if not distinct(a,act) and not distinct(b,act): continue
        n+=1; ok+=match(a,b,act)
    return n==0 or ok>=n*share
def edge(A,B,act,rng):
    last=0
    for c,i in enumerate(rng):
        if not distinct(B[i],act): continue
        if same_row(A[i],B[i],act): last=c+1
        else: break
    return last
BAND=40
def find_offset(A,B,expected):
    h=len(A); act=active_mask(A,B)
    if len(act)<4: return 'END'
    top=edge(A,B,act,range(0,h//3)); bot=edge(A,B,act,range(h-1,h-1-h//3,-1))
    dist=[distinct(r,act) for r in B]
    maxdy=h-top-bot-MinOverlap
    clusters={}
    for b0 in range(top, h-bot, BAND):
        rows=[i for i in range(b0, min(b0+BAND, h-bot)) if dist[i]]
        if len(rows)<6: continue
        best=[]   # (votes, err, dy)
        for dy in range(0, maxdy+1):
            if rows[-1]+dy>=h: break
            v=0; e=0.0
            for i in rows:
                sh,er=row_err(A[i+dy],B[i],act)
                if sh>=0.75: v+=1; e+=er
            if v: best.append((v, e/v, dy))
        if not best: continue
        vmax=max(c[0] for c in best)
        if vmax<0.6*len(rows): continue
        top_c=sorted([c for c in best if c[0]>=0.9*vmax], key=lambda c:c[1])
        dyb=top_c[0][2]
        # mehrdeutig? ein deutlich anderer Kandidat ähnlich gut -> Band enthält sich
        if any(abs(c[2]-dyb)>2 and c[1]<=top_c[0][1]+1.0 for c in top_c[1:]): continue
        clusters.setdefault(dyb,0); clusters[dyb]+=top_c[0][0]
    if not clusters: return None
    # benachbarte dy (±1) zusammenfassen
    tot={d: sum(v for d2,v in clusters.items() if abs(d2-d)<=1) for d in clusters}
    dy=max(tot, key=lambda d:(tot[d], clusters[d], -abs(d-expected)))
    LAST.update(dy=dy, clusters=dict(clusters), top=top, bot=bot)
    if dy==0: return 'END'
    return dy,top,bot

def make_page(N, blank):
    rows=[]
    while len(rows)<N:
        kind=random.random()
        if kind<blank: rows += [[250]*K]*random.randint(5,40)
        elif kind<blank+0.15:   # Bild: weicher Verlauf
            if random.random()<0.3:   # einfarbige Fläche (Himmel, Hintergrund)
                c=random.randint(100,240); rows += [[c]*K]*random.randint(40,150)
            else:                     # Foto: Struktur, die sich über wenige Zeilen ändert
                v=[random.randint(30,220) for _ in range(K)]
                for j in range(random.randint(40,200)):
                    v=[max(0,min(255,int(0.75*x+0.25*random.randint(30,220)))) for x in v]
                    rows.append(v)
        else:                   # Textzeilen
            for j in range(random.randint(10,18)): rows.append([random.choice([250,250,250,120,60,200]) for _ in range(K)])
            rows += [[250]*K]*random.randint(4,10)
    return rows[:N]

def run(page, H, hdr_n, ftr_n, ppn, noise, translucent, anim, sidebar=0, banner=0):
    ch=H-hdr_n-ftr_n; maxs=len(page)-ch
    hdr_base=[[random.randint(0,255) for _ in range(K)] for _ in range(hdr_n)]
    ftr=[[random.randint(0,255) for _ in range(K)] for _ in range(ftr_n)]
    st={'s':0}
    def view():
        s=st['s']; rows=[]; ids=[]
        for i in range(hdr_n):
            r=hdr_base[i]
            if translucent:
                under=page[min(len(page)-1,s+i)]; r=[int(0.7*a+0.3*b) for a,b in zip(r,under)]
            rows.append(r); ids.append(('H',i))
        for i in range(ch):
            r=page[s+i]; idx=s+i
            if anim and hdr_n+ch//2 <= hdr_n+i < hdr_n+ch//2+anim:   # fixierte Animation über dem Inhalt
                r=[random.randint(0,255) for _ in range(K)]
            if noise: r=[max(0,min(255,v+random.randint(-noise,noise))) for v in r]
            if sidebar: r=[ (40 if (i//7)%2 else 90) ]*sidebar + r[sidebar:]
            if banner and i>=ch-banner:
                rows.append([max(0,min(255,60+random.randint(-20,20))) for _ in range(K)]); ids.append(('B',i)); continue
            rows.append(r); ids.append(idx)
        for i in range(ftr_n): rows.append(ftr[i]); ids.append(('F',i))
        return rows, ids
    prev,pid=view(); segs=[]; start=0; center=-1; notches=1; expected=H//3; still=0; known_ppn=None; last_top=last_bot=0
    for step in range(200):
        st['s']=min(maxs, st['s']+notches*ppn); cur,cid=view()
        if nearly_same(prev,cur):
            still+=1
            if still>=2: break
            continue
        still=0
        m=find_offset(prev,cur,expected)
        if m=='END':
            still+=1
            if still>=2: break
            continue
        if m is None and notches>1:
            st['s']=max(0,st['s']-notches*ppn); notches=max(1,notches//2); expected//=2; prev,pid=view(); continue
        if m is None:
            cur,cid=view()
            if nearly_same(prev,cur,0.6): break
            m=find_offset(prev,cur,expected)
            if m=='END': break
        if m is None:
            # Sicherheitsnetz: bekannte Schrittweite verwenden (Browser scrollen je Raste gleich weit)
            if known_ppn: m=(int(round(known_ppn*notches)), last_top, last_bot); FALLBACKS[0]+=1
            else: return None,"keine Zuordnung"
        dy,top,bot=m
        if known_ppn is None or abs(dy/notches-known_ppn)<2: known_ppn=dy/notches
        last_top,last_bot=top,bot
        if center<0: center=(top+H-bot)//2
        seam=max(start,min(H,center+dy//2))
        segs.append(pid[start:seam]); start=max(0,seam-dy)
        p=dy/notches; notches=max(1,min(15,int((H-top-bot)*0.4/max(1,p)))); expected=int(p*notches)
        prev,pid=cur,cid
    segs.append(pid[start:])
    return [x for sg in segs for x in sg], None

def check(out, N, hdr_n, ftr_n):
    c=[x for x in out if isinstance(x,int)]
    if c!=list(range(len(c))) or len(c)<N-60: return False
    # Kopf nur am Anfang, Fuß/Banner nur am Ende (keine Streifen dazwischen)
    first_int=next(i for i,x in enumerate(out) if isinstance(x,int))
    last_int=max(i for i,x in enumerate(out) if isinstance(x,int))
    if any(not isinstance(x,int) for x in out[first_int:last_int+1]): return False
    return True

random.seed(7); res={}
for t in range(36):
    H=random.choice([500,700]); hdr=random.choice([0,60]); ftr=random.choice([0,0,40])
    noise=random.choice([0,3]); trans=hdr>0 and random.random()<0.6; anim=random.choice([0,0,40])
    blank=random.choice([0.2,0.5]); N=random.randint(H*3,H*8)
    page=make_page(N,blank)
    side=random.choice([0,0,6]); ban=random.choice([0,30])
    out,err=run(page,H,hdr,ftr,random.choice([100,125,150]),noise,trans,anim,side,ban)
    ok = out is not None and check(out,N,hdr,ftr)
    key=f"Rauschen={noise} transp.Kopf={trans} Animation={anim>0} Seitenleiste={side>0} Banner={ban>0}"
    res.setdefault(key,[0,0]); res[key][0]+=ok; res[key][1]+=1
    if not ok:
        c=None if out is None else [x for x in out if isinstance(x,int)]
        why=err or (f"{len(c)} Zeilen statt {N}" + ("" if not c else f", erste Abweichung bei {next((i for i,(a,b) in enumerate(zip(c,range(N))) if a!=b), min(len(c),N))}"))
        print("FAIL",t,key,f"H={H}",why)
for k,(a,b) in sorted(res.items()): print(f"{a}/{b}  {k}")
print("Sicherheitsnetz genutzt:",FALLBACKS[0]); print("GESAMT", sum(v[0] for v in res.values()),"/",sum(v[1] for v in res.values()))
