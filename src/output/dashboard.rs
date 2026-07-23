use crate::graph::CodeGraph;
use crate::graph::scc::SccAnalysis;
use crate::output::graph::GraphOutput;

/// Generate an interactive HTML dashboard from graph analysis data.
///
/// Cytoscape.js is always loaded from a CDN so the library is never shipped
/// in the binary; the browser caches it after the first fetch.
pub fn to_graph_html(
    graph: &CodeGraph,
    scc_analysis: &SccAnalysis,
    snapshot_id: u64,
) -> anyhow::Result<String> {
    let graph_output = GraphOutput::from_graph(graph, Some(scc_analysis), snapshot_id);
    let json_data = serde_json::to_string(&graph_output)?;

    let html = HTML_TEMPLATE
        .replacen("__CDN_SCRIPT__", &cdn_script(), 1)
        .replacen("__DATA__", &json_data, 1);
    Ok(html)
}

fn cdn_script() -> String {
    r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/cytoscape/3.30.4/cytoscape.min.js"></script>"#.to_string()
}

const HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Meta-AST Graph Dashboard</title>
<style>
*,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
body{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Oxygen,Ubuntu,sans-serif;background:#1a1a2e;color:#e0e0e0;height:100vh;display:flex;flex-direction:column}
#header{display:flex;align-items:center;justify-content:space-between;padding:8px 16px;background:#16213e;border-bottom:1px solid #0f3460;flex-shrink:0}
#header h1{font-size:14px;font-weight:600;color:#e94560}
#header h1 a{color:#e94560;text-decoration:none}
#header h1 a:hover{text-decoration:underline}
#header #stats{font-size:12px;color:#8899aa}
#toolbar{display:flex;align-items:center;gap:8px;padding:6px 16px;background:#16213e;border-bottom:1px solid #0f3460;flex-shrink:0;flex-wrap:wrap}
#toolbar button,#toolbar select{background:#0f3460;color:#e0e0e0;border:1px solid #1a4a7a;border-radius:4px;padding:4px 10px;font-size:12px;cursor:pointer}
#toolbar button:hover{background:#1a4a7a}
#toolbar select{background:#0f3460;cursor:pointer}
#toolbar label{font-size:12px;color:#8899aa}
#main{display:flex;flex:1;min-height:0}
#cy{flex:1;min-width:0}
#sidebar{width:280px;background:#16213e;border-left:1px solid #0f3460;padding:12px;overflow-y:auto;flex-shrink:0;font-size:13px}
#sidebar h3{font-size:13px;color:#e94560;margin-bottom:8px;text-transform:uppercase;letter-spacing:0.5px}
#sidebar pre{font-family:"SFMono-Regular",Consolas,"Liberation Mono",Menlo,monospace;font-size:12px;white-space:pre-wrap;word-break:break-all;color:#c0c0c0}
#sidebar .empty{color:#667;font-style:italic}
#legend{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px;padding-top:12px;border-top:1px solid #0f3460}
#legend .item{display:flex;align-items:center;gap:6px;font-size:11px}
#legend .dot{width:12px;height:12px;border-radius:50%;flex-shrink:0}
.legend-cyclic{background:#e94560}
.legend-independent{background:#2ecc71}
.legend-acyclic{background:#3498db}
.legend-selfloop{background:#f39c12}
.legend-file{background:#9b59b6;border-radius:2px}
.legend-symbol{background:#ecf0f1}
</style>
</head>
<body>
<div id="header">
<h1><a href="https://github.com/metacall">MetaCall GSoC 2026</a> - Meta-AST Graph Dashboard</h1>
<span id="stats">Loading...</span>
<a href="https://discord.gg/VvSZRsBK" style="color:#7289da;font-size:12px;text-decoration:none" target="_blank">Join us on Discord</a>
</div>
<div id="toolbar">
<button onclick="cy.zoom(cy.zoom()*1.3)" title="Zoom in">+ Zoom In</button>
<button onclick="cy.zoom(cy.zoom()/1.3)" title="Zoom out">- Zoom Out</button>
<button onclick="cy.fit()" title="Fit graph to view">&larr;&rarr; Fit</button>
<span style="color:#667">|</span>
<label for="layout-select">Layout:</label>
<select id="layout-select" onchange="changeLayout(this.value)">
<option value="cose">Force-Directed</option>
<option value="breadthfirst">Hierarchical</option>
<option value="concentric">Concentric</option>
<option value="grid">Grid</option>
<option value="circle">Circle</option>
</select>
<span style="color:#667">|</span>
<button onclick="highlightSCCs()">Highlight SCCs</button>
<button onclick="resetStyle()">Reset View</button>
<button onclick="exportPNG()">Export PNG</button>
</div>
<div id="main">
<div id="cy"></div>
<div id="sidebar">
<h3>Node Details</h3>
<pre id="details" class="empty">Click a node to inspect</pre>
<div id="legend">
<div class="item"><div class="dot legend-cyclic"></div>Cyclic (refactor)</div>
<div class="item"><div class="dot legend-independent"></div>Independent</div>
<div class="item"><div class="dot legend-acyclic"></div>Acyclic Dep</div>
<div class="item"><div class="dot legend-selfloop"></div>Self-Loop</div>
<div class="item"><div class="dot legend-file"></div>File</div>
<div class="item"><div class="dot legend-symbol"></div>Symbol</div>
</div>
</div>
</div>
<script>
var DATA=__DATA__;
window.addEventListener("DOMContentLoaded",function(){
var cy=window.cy=cytoscape({
container:document.getElementById("cy"),
elements:buildElements(DATA),
style:buildStyle(),
layout:{name:"cose",padding:50,nodeRepulsion:8000,idealEdgeLength:120,animate:true},
wheelSensitivity:0.3
});
updateStats(DATA);
cy.on("tap","node",function(evt){
var n=evt.target;
var data=n.data();
var sid=data.scc_index;
var scc=(sid!==undefined&&DATA.sccs[sid])?DATA.sccs[sid]:null;
var details=document.getElementById("details");
details.textContent="";
var lines=[];
lines.push({b:"Name: ",t:data.name||"(root)"});
lines.push({b:"Kind: ",t:data.kind});
if(data.path)lines.push({b:"Path: ",t:data.path});
if(data.symbol_kind)lines.push({b:"Symbol Kind: ",t:data.symbol_kind});
if(data.visibility)lines.push({b:"Visibility: ",t:data.visibility});
if(data.language)lines.push({b:"Language: ",t:data.language});
if(scc){
  lines.push({b:"",t:"",br:true});
  lines.push({b:"SCC #"+scc.index,t:"",br:false});
  lines.push({b:"Size: ",t:""+scc.size});
  lines.push({b:"Cyclic: ",t:scc.is_cyclic?"Yes (refactor needed)":"No"});
  lines.push({b:"Hint: ",t:scc.hint});
}
lines.forEach(function(l){
  if(l.br){details.appendChild(document.createElement("br"));return;}
  var b=document.createElement("b");b.textContent=l.b;details.appendChild(b);
  var s=document.createTextNode(l.t);details.appendChild(s);
  details.appendChild(document.createElement("br"));
});
details.className="";
});
cy.on("tap",function(evt){
if(evt.target===cy){document.getElementById("details").textContent="Click a node to inspect";document.getElementById("details").className="empty";}
});
window.cy=cy;
});
function buildElements(data){
var sccMap={};
data.sccs.forEach(function(s,si){s.nodes.forEach(function(ni){sccMap[ni]=si;});});
var els=[];
data.nodes.forEach(function(n,i){
var si=sccMap[i];
var s=(si!==undefined&&data.sccs[si])?data.sccs[si]:null;
var cls="scc-normal";
var hint="";
if(s){
if(s.hint==="cyclic_cluster")cls="scc-cyclic";
else if(s.hint==="self_loop")cls="scc-selfloop";
else if(s.hint==="independent")cls="scc-independent";
else if(s.hint==="acyclic_dependency")cls="scc-acyclic";
hint=s.hint;
}
els.push({
data:{
id:"n"+i,
name:n.name||n.path||"",
kind:n.kind,
path:n.path,
symbol_kind:n.symbol_kind,
visibility:n.visibility,
language:n.language,
scc_index:si,
scc_hint:hint,
scc_class:cls
},
classes:cls
});
});
data.edges.forEach(function(e,i){
els.push({
data:{
id:"e"+i,
source:"n"+e.source,
target:"n"+e.target,
kind:e.kind
},
classes:"edge-"+e.kind
});
});
return els;
}
function buildStyle(){
return [
{selector:"node",style:{
"background-color":"#7f8c8d",label:"data(name)",
"font-size":"10px","text-valign":"bottom","text-halign":"center",
"text-wrap":"wrap","text-max-width":"120px",color:"#ccc",
"border-width":1,"border-color":"#555","width":20,"height":20
}},
{selector:"node.scc-cyclic",style:{"background-color":"#e94560","border-color":"#ff6b81","width":28,"height":28}},
{selector:"node.scc-independent",style:{"background-color":"#2ecc71","border-color":"#55efc4"}},
{selector:"node.scc-acyclic",style:{"background-color":"#3498db","border-color":"#74b9ff"}},
{selector:"node.scc-selfloop",style:{"background-color":"#f39c12","border-color":"#fdcb6e"}},
{selector:'node[kind="file"]',style:{shape:"round-rectangle","border-width":2,"border-color":"#9b59b6","background-color":"#8e44ad",width:30,height:24,"text-valign":"center","text-halign":"center"}},
{selector:'node[kind="symbol"]',style:{shape:"ellipse"}},
{selector:"edge",style:{width:1,"line-color":"#555","target-arrow-color":"#555","target-arrow-shape":"triangle","curve-style":"bezier","arrow-scale":0.7}},
{selector:"edge.edge-ownership",style:{"line-color":"#9b59b6","target-arrow-color":"#9b59b6","width":1.5,"line-style":"dotted"}},
{selector:"edge.edge-import",style:{"line-color":"#3498db","target-arrow-color":"#3498db","width":1.5}},
{selector:"edge.edge-reference",style:{"line-color":"#e67e22","target-arrow-color":"#e67e22","width":1}},
{selector:":selected",style:{"border-color":"#fff","border-width":3}}
];
}
function updateStats(data){
var nFiles=data.nodes.filter(function(n){return n.kind==="file";}).length;
var nSymbols=data.nodes.filter(function(n){return n.kind==="symbol";}).length;
var nCyclic=data.sccs.filter(function(s){return s.is_cyclic;}).length;
var nIndep=data.sccs.filter(function(s){return s.hint==="independent"||s.hint==="acyclic_dependency";}).length;
document.getElementById("stats").textContent=
data.nodes.length+" nodes | "+data.edges.length+" edges | "+
data.sccs.length+" SCCs | "+nCyclic+" cyclic | "+nIndep+" deployable";
}
function changeLayout(name){
var opts={name:name,padding:50,animate:true};
if(name==="cose"){opts.nodeRepulsion=8000;opts.idealEdgeLength=120;}
cy.layout(opts).run();
}
function highlightSCCs(){
var cyclic=DATA.sccs.filter(function(s){return s.is_cyclic;});
var nIds=new Set();
cyclic.forEach(function(s){s.nodes.forEach(function(ni){nIds.add("n"+ni);});});
cy.nodes().forEach(function(n){if(nIds.has(n.id())){n.style({"border-color":"#fff","border-width":4});}else{n.style({opacity:0.3});}});
}
function resetStyle(){
cy.nodes().forEach(function(n){n.style({"border-color":"","border-width":"","opacity":1});});
}
function exportPNG(){
var png=cy.png({full:true,scale:2});
var a=document.createElement("a");a.href=png;a.download="meta-ast-graph.png";a.click();
}
</script>
__CDN_SCRIPT__
</body>
</html>"##;
