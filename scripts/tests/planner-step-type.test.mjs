import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source=(await readFile(new URL('../../web/modules/deployments/planner-steps.js',import.meta.url),'utf8')).replace(/^import .*;\n/,'').replaceAll('export ','');
const escapeHtml=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const {mountPlannerSteps}=new Function('escapeHtml','CSS',source+'\nreturn {mountPlannerSteps};')(escapeHtml,{escape:s=>s});
class Element {
 constructor(){this.listeners={};this.value='';}
 addEventListener(name,fn){this.listeners[name]=fn;}
 fire(name,value){this.value=value;this.listeners[name]?.({target:this});}
 focus(){}
}
class Root {
 set innerHTML(html){
  this.html=html;this.top=new Map(['data-add-step','data-planner-strategy','data-parallel-regions'].map(k=>[k,new Element()]));
  this.articles=[...html.matchAll(/<article[^>]*data-planner-step="([^"]+)"[^>]*>([\s\S]*?)<\/article>/g)].map(([,id,body])=>{
   const controls=new Map([...body.matchAll(/\b(data-step-[a-z-]+)/g)].map(m=>[m[1],new Element()]));
   return {dataset:{plannerStep:id},querySelector:selector=>controls.get(selector.slice(1,-1))??null};
  });
 }
 querySelector(selector){return this.top.get(selector.slice(1,-1))??null;}
 querySelectorAll(){return this.articles;}
 replaceChildren(){this.articles=[];}
}
test('switching step types shows only the applicable editor and preserves both drafts',()=>{
 const draft={steps:[{id:'one',name:'Check',extraVarsText:'VERSION: 3'}]};const root=new Root();
 const queries=[{id:'q',name:'Saved',query:'metric{version="${EXPECTED_VERSION}"}'}];
 const dispose=mountPlannerSteps(root,{draft,validationQueries:()=>queries});
 assert.ok(root.articles[0].querySelector('[data-step-vars]'));
 root.articles[0].querySelector('[data-step-type]').fire('change','thanos_query');
 assert.equal(root.articles[0].querySelector('[data-step-vars]'),null);
 root.articles[0].querySelector('[data-step-saved-query]').fire('change','q');
 root.articles[0].querySelector('[data-step-expected-version]').fire('input','1.97.0');
 assert.equal(draft.steps[0].expectedVersion,'1.97.0');
 assert.equal(draft.steps[0].query,queries[0].query);
 root.articles[0].querySelector('[data-step-type]').fire('change','deploy');
 assert.ok(root.articles[0].querySelector('[data-step-vars]'));assert.equal(draft.steps[0].extraVarsText,'VERSION: 3');
 root.articles[0].querySelector('[data-step-type]').fire('change','thanos_query');
 assert.equal(draft.steps[0].query,queries[0].query);
 assert.equal(draft.steps[0].expectedVersion,'1.97.0');
 assert.ok(!root.html.includes('data-step-timeout'));assert.ok(!root.html.includes('data-step-interval'));
 dispose();assert.equal(root.articles.length,0);
});


test('saved query selection copies PromQL and follows library edits without overwriting expected version',()=>{
 const queries=[{id:'q',name:'Exporter version',role:'exporter',query:'metric{version="${EXPECTED_VERSION}"}'}];
 const draft={steps:[{id:'one',name:'',type:'thanos_query',query:'',expectedVersion:'1.97.0'}]};const root=new Root();
 const dispose=mountPlannerSteps(root,{draft,validationQueries:()=>queries});
 assert.ok(root.html.includes('>Exporter version</option>'));assert.ok(!root.html.includes('Exporter version · exporter'));
 root.articles[0].querySelector('[data-step-saved-query]').fire('change','q');
 assert.equal(draft.steps[0].query,queries[0].query);
 assert.equal(draft.steps[0].name,'Thanos');
 assert.equal(draft.steps[0].expectedVersion,'1.97.0');
 queries[0].query='vector(0)';dispose.refresh();
 assert.equal(draft.steps[0].query,'vector(0)');
 assert.equal(draft.steps[0].expectedVersion,'1.97.0');
 assert.equal(root.articles[0].querySelector('[data-step-query]'),null);
 assert.ok(root.html.includes('data-step-query-preview'));assert.ok(root.html.includes('readonly'));
 assert.ok(!root.html.includes('Custom query'));
 assert.equal(draft.steps[0].validationQueryId,'q');
 dispose();
});

test('parallel region control defaults to one and keeps its value across step edits',()=>{
 const draft={steps:[{id:'one',name:'Deploy',extraVarsText:''}]}; const root=new Root();
 mountPlannerSteps(root,{draft}); assert.equal(draft.parallelRegions,1);
 assert.ok(root.html.includes('aria-label="Parallel regions"'));
 root.querySelector('[data-parallel-regions]').fire('input','3');
 root.articles[0].querySelector('[data-step-type]').fire('change','thanos_query');
 assert.equal(draft.parallelRegions,'3'); assert.ok(root.html.includes('value="3"'));
});

test('copied query matches its saved template and a missing library provides no custom editor',()=>{
 const query='vector(1)';const root=new Root();
 const draft={steps:[{id:'one',name:'Thanos Query',type:'thanos_query',query}]};
 const dispose=mountPlannerSteps(root,{draft,validationQueries:()=>[{id:'saved',name:'Check',query}]});
 assert.equal(draft.steps[0].validationQueryId,'saved');
 assert.equal(root.articles[0].querySelector('[data-step-query]'),null);
 assert.ok(root.html.includes('readonly'));assert.ok(!root.html.includes('data-step-expected-version'));
 dispose();
 const empty=new Root();mountPlannerSteps(empty,{draft:{steps:[{id:'two',name:'Thanos Query',type:'thanos_query'}]}});
 assert.ok(empty.html.includes('Add queries in Config → Thanos Queries'));
 assert.ok(!empty.html.includes('Custom query'));
});

test('step names default to their type and follow type changes while custom names are preserved',()=>{
 const draft={steps:[]};const root=new Root();mountPlannerSteps(root,{draft});
 assert.equal(draft.steps[0].name,'Deploy');
 root.articles[0].querySelector('[data-step-type]').fire('change','thanos_query');
 assert.equal(draft.steps[0].name,'Thanos');assert.match(root.html,/value="Thanos"/);
 root.articles[0].querySelector('[data-step-type]').fire('change','deploy');
 assert.equal(draft.steps[0].name,'Deploy');
 root.articles[0].querySelector('[data-step-name]').fire('input','Custom rollout');
 root.articles[0].querySelector('[data-step-type]').fire('change','thanos_query');
 assert.equal(draft.steps[0].name,'Custom rollout');
 root.articles[0].querySelector('[data-step-name]').fire('input','');
 root.articles[0].querySelector('[data-step-type]').fire('change','deploy');
 assert.equal(draft.steps[0].name,'Deploy');
 root.querySelector('[data-add-step]').fire('click');
 assert.equal(draft.steps[1].name,'Deploy');
});
