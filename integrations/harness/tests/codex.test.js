import test from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import {mkdtempSync,writeFileSync,rmSync,symlinkSync} from 'node:fs';
import {executeTool,toolContracts,verifyIdentity} from '../src/codex.js';
const config={localNode:'test-node',identity:{account:os.userInfo().username,hostnames:[os.hostname()]},binary:process.env.SWE_GREP_BINARY??'/missing-binary',routes:{}};
test('ambiguous target, relative path, and unavailable nodes fail explicitly',async()=>{
 for(const args of [{query:'x',path:'/tmp'},{query:'x',path:'.',node:'test-node'},{query:'x',path:'/tmp',node:'elsewhere'}])
  await assert.rejects(executeTool(config,'swe_search',args),error=>['SWE_GREP_INVALID_INPUT','SWE_GREP_NODE_UNAVAILABLE'].includes(error.code));
 await assert.rejects(executeTool(config,'swe_search',{query:'x',path:'/tmp',node:'test-node',binary:'/other'}),/Unknown argument/);
});
test('identity drift is rejected and model tool schemas require target and path',()=>{
 assert.throws(()=>verifyIdentity({...config,identity:{...config.identity,account:'wrong-account'}}),/Hostname or account/);
 for(const tool of toolContracts(config)){assert.deepEqual(tool.inputSchema.required,['query','path','node']);assert.deepEqual(tool.inputSchema.properties.node.enum,['test-node']);}
});
test('all three tools work outside home with canonical node-tagged results', {skip:!process.env.SWE_GREP_BINARY}, async()=>{
 const temp=mkdtempSync(path.join(os.tmpdir(),'swe-codex-any-path-'));
 try{
 writeFileSync(path.join(temp,'fixture.py'),'def any_path_target():\n    return 1\n');
 for(const name of ['swe_search','swe_context','swe_refs']){
 const result=await executeTool(config,name,{node:'test-node',path:temp,query:'any_path_target'});
 assert.match(JSON.stringify(result),/any_path_target/);assert.equal(result._swe.node,'test-node');
 }
 if(process.platform!=='win32'){
 symlinkSync(temp,path.join(temp,'link'));
 const result=await executeTool(config,'swe_search',{node:'test-node',path:path.join(temp,'link'),query:'any_path_target'});
 assert.ok(!result._swe.path.endsWith('/link'));
 }
 }finally{rmSync(temp,{recursive:true,force:true});}
});

test('remote-relative paths and inherited route names are rejected before launch', async () => {
 const remote = {...config, routes:{remote:{command:'must-not-run',args:[]}}};
 await assert.rejects(executeTool(remote,'swe_search',{node:'remote',path:'relative/file.py',query:'x'}), {code:'SWE_GREP_INVALID_INPUT'});
 await assert.rejects(executeTool(remote,'swe_search',{node:'toString',path:'/tmp',query:'x'}), {code:'SWE_GREP_NODE_UNAVAILABLE'});
 const controller = new AbortController(); controller.abort();
 await assert.rejects(executeTool(remote,'swe_search',{node:'remote',path:'/tmp',query:'x'},{signal:controller.signal}), {name:'AbortError'});
});

test('node contracts retain directory-only context and glob-array guidance', () => {
 const tools=toolContracts(config);
 assert.match(tools.find(x=>x.name==='swe_context').inputSchema.properties.path.description,/directory/);
 assert.match(tools.find(x=>x.name==='swe_search').inputSchema.properties.pathGlob.description,/array even for one/);
});

test('missing local paths return the shared path error instead of raw filesystem errors', async () => {
 const temp=mkdtempSync(path.join(os.tmpdir(),'swe-codex-missing-'));
 try {
  await assert.rejects(executeTool(config,'swe_search',{
   node:'test-node',path:path.join(temp,'does-not-exist.py'),query:'x',
  }), {code:'SWE_GREP_INVALID_PATH'});
 } finally {rmSync(temp,{recursive:true,force:true});}
});
