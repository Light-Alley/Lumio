<script lang="ts">
  import { onMount, untrack } from "svelte";
  import L from "leaflet";
  import "leaflet/dist/leaflet.css";
  import "leaflet.markercluster/dist/MarkerCluster.css";
  import "leaflet.markercluster/dist/MarkerCluster.Default.css";
  import { Search, MapPin, Crosshair } from "@lucide/svelte";
  import { app, selection, clickSelect, applyEdit, openViewer, selectedIds, withStacks, confirm, errorToast } from "../store.svelte";
  import { api, thumbUrl, type PhotoLite, type Place } from "../api";
  import { showMenu } from "../menu.svelte";
  import { photoMenu } from "../actions";

  const TILES: Record<string, { url: string; attribution: string; maxZoom: number }> = {
    osm: { url: "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", attribution: "© OpenStreetMap contributors", maxZoom: 19 },
    carto: { url: "https://{s}.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}{r}.png", attribution: "© OpenStreetMap contributors © CARTO", maxZoom: 20 },
    cartodark: { url: "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png", attribution: "© OpenStreetMap contributors © CARTO", maxZoom: 20 },
    satellite: { url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}", attribution: "Tiles © Esri, Maxar, Earthstar Geographics", maxZoom: 19 },
    topo: { url: "https://{s}.tile.opentopomap.org/{z}/{x}/{y}.png", attribution: "© OpenStreetMap contributors, SRTM | © OpenTopoMap (CC-BY-SA)", maxZoom: 17 },
  };

  let mapEl: HTMLDivElement | undefined = $state();
  let map: L.Map | null = null;
  let cluster: L.MarkerClusterGroup | null = null;
  let tileLayer: L.TileLayer | null = null;
  let placing = $state(false);
  let query = $state("");
  let results = $state<Place[]>([]);
  let dropHover = $state(false);
  let fitted = false;
  let ready = $state(false);

  const geo = $derived(app.photos.filter((p) => p.lat != null && p.lon != null));
  const strip = $derived(app.photos.slice(0, 600));

  function icon(p: PhotoLite, sel: boolean) {
    return L.divIcon({
      className: "",
      html: `<div class="pin${sel ? " sel" : ""}"><img src="${thumbUrl(p)}" alt=""/></div>`,
      iconSize: [46, 46],
      iconAnchor: [23, 46],
    });
  }

  function setTiles() {
    if (!map) return;
    const t = TILES[app.settings.map_tiles ?? "osm"] ?? TILES.osm;
    tileLayer?.remove();
    tileLayer = L.tileLayer(t.url, { attribution: t.attribution, maxZoom: t.maxZoom, referrerPolicy: "strict-origin-when-cross-origin" } as L.TileLayerOptions).addTo(map);
  }

  onMount(() => {
    let alive = true;
    // leaflet.markercluster expects a global `L`.
    (window as unknown as { L: typeof L }).L = L;
    import("leaflet.markercluster").then(() => { if (alive) setup(); });
    return () => { alive = false; map?.remove(); map = null; };
  });

  function setup() {
    map = L.map(mapEl!, { zoomControl: true, worldCopyJump: true }).setView([20, 0], 2);
    setTiles();
    cluster = L.markerClusterGroup({
      showCoverageOnHover: false,
      maxClusterRadius: 50,
      iconCreateFunction: (c) => {
        const ms = c.getAllChildMarkers();
        const p = (ms[0].options as { photo?: PhotoLite }).photo!;
        return L.divIcon({
          className: "",
          html: `<div class="pin cl"><img src="${thumbUrl(p)}" alt=""/><span>${ms.length}</span></div>`,
          iconSize: [52, 52],
          iconAnchor: [26, 52],
        });
      },
    });
    map.addLayer(cluster);
    map.on("click", (e: L.LeafletMouseEvent) => {
      if (placing) placeAt(e.latlng.lat, e.latlng.lng, selectedIds());
    });
    ready = true;
  }

  $effect(() => {
    app.settings.map_tiles;
    if (ready) setTiles();
  });

  // Markers are made when the photos change; a selection change only redraws the
  // markers whose state changed, not every marker and cluster.
  let markers = new Map<number, { m: L.Marker; p: PhotoLite; on: boolean }>();
  $effect(() => {
    const list = geo;
    if (!ready || !cluster || !map) return;
    const sel = untrack(() => new Set(selection));
    cluster.clearLayers();
    markers = new Map();
    const made = list.map((p) => {
      const on = sel.has(p.id);
      const m = L.marker([p.lat!, p.lon!], { icon: icon(p, on), photo: p } as L.MarkerOptions & { photo: PhotoLite });
      m.on("click", (e) => clickSelect(p.id, (e as L.LeafletMouseEvent).originalEvent));
      m.on("dblclick", () => openViewer(p.id));
      m.on("contextmenu", (e) => showMenu((e as L.LeafletMouseEvent).originalEvent, photoMenu(p)));
      markers.set(p.id, { m, p, on });
      return m;
    });
    cluster.addLayers(made);
    if (!fitted && list.length) {
      fitted = true;
      map.fitBounds(L.latLngBounds(list.map((p) => [p.lat!, p.lon!] as [number, number])), { padding: [40, 40], maxZoom: 14 });
    }
  });
  $effect(() => {
    const sel = new Set(selection);
    for (const entry of markers.values()) {
      const on = sel.has(entry.p.id);
      if (on !== entry.on) {
        entry.on = on;
        entry.m.setIcon(icon(entry.p, on));
      }
    }
  });

  async function placeAt(lat: number, lon: number, ids: number[]) {
    if (!ids.length) return;
    const ok = await confirm(
      `Set the location of ${ids.length} photo${ids.length > 1 ? "s" : ""}?`,
      `${lat.toFixed(5)}, ${lon.toFixed(5)}. The GPS position is written into the photo files (or their XMP sidecars for RAW files).`,
      "Set location",
    );
    if (!ok) return;
    placing = false;
    await applyEdit(ids, { gps: [lat, lon] });
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dropHover = false;
    const data = e.dataTransfer?.getData("application/x-lumio-photos");
    if (!data || !map) return;
    const ll = map.mouseEventToLatLng(e as unknown as MouseEvent);
    placeAt(ll.lat, ll.lng, JSON.parse(data));
  }

  async function search() {
    if (!query.trim()) return;
    try {
      results = await api.geocode(query);
      if (results[0]) go(results[0]);
    } catch (e) {
      errorToast(e);
    }
  }
  function go(p: Place) {
    map?.setView([p.lat, p.lon], 13);
    results = [];
  }

  function stripDrag(e: DragEvent, p: PhotoLite) {
    if (!selection.has(p.id)) clickSelect(p.id, e as unknown as MouseEvent);
    // Closed stacks are dragged whole, so every photo in them gets the place.
    e.dataTransfer?.setData("application/x-lumio-photos", JSON.stringify(withStacks(selectedIds())));
  }
</script>

<div class="mapview">
  <div class="mapwrap" class:placing class:drophover={dropHover}
    ondragover={(e) => { e.preventDefault(); dropHover = true; }}
    ondragleave={() => (dropHover = false)}
    ondrop={onDrop}
    role="application"
  >
    <div class="map" bind:this={mapEl}></div>
    <div class="overlay">
      <div class="search">
        <Search size={14} />
        <input type="search" placeholder="Find a place…" bind:value={query} onkeydown={(e) => e.key === "Enter" && search()} />
        {#if results.length > 1}
          <div class="results">
            {#each results as r}<button class="ghost" onclick={() => go(r)}>{r.name}</button>{/each}
          </div>
        {/if}
      </div>
      <button class:primary={placing} disabled={!selection.size} onclick={() => (placing = !placing)} title="Click on the map to set the location of the selected photos">
        <Crosshair size={14} /> {placing ? "Click the map to place…" : `Place ${selection.size || ""} selected`}
      </button>
    </div>
    <div class="hint">{geo.length.toLocaleString()} of {app.photos.length.toLocaleString()} photos have a location. Drag photos from the strip below onto the map to geotag them.</div>
  </div>
  <div class="strip">
    {#each strip as p (p.id)}
      <button
        class="sthumb"
        class:sel={selection.has(p.id)}
        draggable="true"
        ondragstart={(e) => stripDrag(e, p)}
        onclick={(e) => { clickSelect(p.id, e); if (p.lat != null) map?.setView([p.lat, p.lon!], Math.max(map.getZoom(), 12)); }}
        ondblclick={() => openViewer(p.id)}
        oncontextmenu={(e) => showMenu(e, photoMenu(p))}
        title={p.filename}
      >
        {#if p.thumb}<img src={thumbUrl(p)} alt="" draggable="false" loading="lazy" />{/if}
        {#if p.lat != null}<span class="has"><MapPin size={10} /></span>{/if}
      </button>
    {/each}
    {#if app.photos.length > strip.length}<div class="more faint">+{(app.photos.length - strip.length).toLocaleString()} more. Narrow the view to see them.</div>{/if}
  </div>
</div>

<style>
  .mapview { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .mapwrap { flex: 1; position: relative; }
  .map { position: absolute; inset: 0; background: var(--bg-3); }
  .mapwrap.placing .map { cursor: crosshair; }
  .mapwrap.drophover::after { content: ""; position: absolute; inset: 0; border: 3px dashed var(--accent); pointer-events: none; z-index: 800; }
  .overlay { position: absolute; top: 10px; left: 56px; right: 10px; z-index: 700; display: flex; gap: 8px; align-items: flex-start; }
  .search { position: relative; width: 300px; display: flex; align-items: center; }
  .search > :global(svg) { position: absolute; left: 9px; color: var(--text-3); z-index: 1; }
  .search input { padding-left: 28px; height: 30px; background: var(--bg-2); box-shadow: var(--shadow); }
  .results { position: absolute; top: 34px; left: 0; right: 0; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 8px; box-shadow: var(--shadow); padding: 4px; }
  .results button { width: 100%; text-align: left; white-space: normal; justify-content: flex-start; }
  .overlay > button { box-shadow: var(--shadow); height: 30px; }
  .hint { position: absolute; bottom: 8px; left: 50%; transform: translateX(-50%); z-index: 700; background: rgba(0,0,0,.65); color: #eee; padding: 4px 12px; border-radius: 12px; font-size: 11.5px; white-space: nowrap; }
  .strip { height: 92px; flex-shrink: 0; display: flex; gap: 6px; padding: 8px; overflow-x: auto; background: var(--bg-2); border-top: 1px solid var(--line); align-items: center; }
  .sthumb { width: 74px; height: 74px; flex-shrink: 0; padding: 0; position: relative; justify-content: center; background: var(--bg-3); border: 2px solid transparent; border-radius: 6px; overflow: hidden; }
  .sthumb.sel { border-color: var(--accent); }
  .sthumb img { max-width: 100%; max-height: 100%; }
  .has { position: absolute; bottom: 2px; right: 2px; background: var(--accent); color: white; border-radius: 50%; width: 15px; height: 15px; display: flex; align-items: center; justify-content: center; }
  .more { white-space: nowrap; padding: 0 10px; }
  :global(.pin) { width: 46px; height: 46px; border-radius: 6px; border: 2px solid white; background: #333; box-shadow: 0 2px 8px rgba(0,0,0,.5); overflow: hidden; position: relative; }
  :global(.pin img) { width: 100% !important; height: 100% !important; object-fit: cover; display: block; }
  :global(.pin.sel) { border-color: #4f8cff; box-shadow: 0 0 0 3px rgba(79,140,255,.6); }
  :global(.pin.cl) { width: 52px; height: 52px; }
  :global(.pin.cl span) { position: absolute; top: 2px; right: 2px; background: #4f8cff; color: white; font: 600 10px system-ui; border-radius: 8px; padding: 1px 5px; }
</style>
