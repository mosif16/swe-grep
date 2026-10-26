/** Codex node router. Reuses the DeepSeek backend; routes are trusted host config. */
import { readFileSync } from 'node:fs';
import { realpath } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { CallToolRequestSchema, ListToolsRequestSchema } from '@modelcontextprotocol/sdk/types.js';
import { TOOLS, validateToolCall } from './contracts.js';
import { SweGrepBackend } from './backend.js';

const fail=(code,message)=>Object.assign(new Error(message),{code});
export function verifyIdentity(config) {
  const identity=config?.identity;
  if (typeof config?.localNode !== 'string' || !config.localNode || typeof config.binary !== 'string')
    throw fail('SWE_GREP_INVALID_CONFIG', 'localNode and an absolute binary are required');
  if(!identity || os.userInfo().username!==identity.account || !Array.isArray(identity.hostnames) || !identity.hostnames.every(x=>typeof x==='string') || !identity.hostnames.some(x=>x.toLowerCase()===os.hostname().toLowerCase()))
    throw fail('SWE_GREP_NODE_IDENTITY_MISMATCH','Hostname or account does not match the configured node');
  if(identity.machineId && readFileSync('/etc/machine-id','utf8').trim()!==identity.machineId)
    throw fail('SWE_GREP_NODE_IDENTITY_MISMATCH','Machine ID does not match the configured node');
  if(!path.isAbsolute(config.binary)) throw new Error('Binary must be absolute');
}

export function toolContracts(config) {
  const nodes=[config.localNode,...Object.keys(config.routes??{})];
  return TOOLS.map(tool=>({...tool,
    description:tool.description+' Select the node already authorized for this task, even after SSH hops. Paths belong to that node. Never substitute another node after a failure. Any path readable by that login is supported; no elevation.',
    inputSchema:{...tool.inputSchema,required:['query','path','node'],properties:{...tool.inputSchema.properties,
      path:{type:'string',description:tool.name==='swe_context'
        ? 'Absolute directory on the selected node. Files are rejected; pass the containing directory explicitly.'
        : 'Absolute file or directory path on the selected node; any readable filesystem location, including mounted volumes. Use a narrow project directory.'},
      node:{type:'string',enum:nodes,description:'Execution node from the user-confirmed task scope. SSH shell commands do not change this MCP connection; select the target here.'}}},
    annotations:{readOnlyHint:true,destructiveHint:false,idempotentHint:true,openWorldHint:false}
  }));
}

export async function executeTool(config,name,args,{signal}={}) {
  verifyIdentity(config);
  signal?.throwIfAborted();
  if(!args || typeof args.node!=='string' || !args.node) throw fail('SWE_GREP_INVALID_INPUT','node is required; use the user-confirmed target');
  if(typeof args.path!=='string' || !args.path) throw fail('SWE_GREP_INVALID_INPUT','An absolute path on the selected node is required');
  if (!path.posix.isAbsolute(args.path) && !path.win32.isAbsolute(args.path))
    throw fail('SWE_GREP_INVALID_INPUT', 'path must be absolute on the selected node');
  const {node,...input}=args;
  const validated=validateToolCall(name,input);
  if(node!==config.localNode){
    const route=config.routes && Object.hasOwn(config.routes,node) ? config.routes[node] : undefined;
    if(!route) throw fail('SWE_GREP_NODE_UNAVAILABLE','No existing trusted route to '+node+' from '+config.localNode+'; invoke SWE-Grep on that node or from the Mac. No local fallback.');
    if (typeof route.command !== 'string' || !Array.isArray(route.args) || route.args.some(x=>typeof x!=='string'))
      throw fail('SWE_GREP_INVALID_CONFIG', 'Routes require a command and string arguments');
    const client=new Client({name:'swe-grep-node-router',version:'0.2.2'});
    const transport=new StdioClientTransport({command:route.command,args:route.args,stderr:'pipe'});
    // Drain diagnostics without exposing SSH/user environment data as search results.
    transport.stderr?.resume();
    try{
      await client.connect(transport,{signal,timeout:15000});
      const result=await client.callTool({name,arguments:args},undefined,{signal,timeout:45000});
      if(result.isError)throw fail(result.structuredContent?.error?.code??'SWE_GREP_REMOTE_ERROR',result.structuredContent?.error?.message??'Remote tool failed');
      if(result.structuredContent?._swe?.node!==node)throw fail('SWE_GREP_NODE_IDENTITY_MISMATCH','Remote response identifies a different node');
      return result.structuredContent;
    }finally{await client.close();}
  }
  if(!path.isAbsolute(validated.path))throw fail('SWE_GREP_INVALID_INPUT','path must be absolute on '+node);
  // Canonicalize first: symlinks, Windows drive letters, and UNC shares select
  // their actual volume root. OS permissions still govern every read.
  let target;
  try {
    target=await realpath(validated.path);
  } catch (cause) {
    signal?.throwIfAborted();
    throw Object.assign(fail('SWE_GREP_INVALID_PATH', 'Cannot resolve requested path'), {cause});
  }
  const root=path.parse(target).root;
  const backend=new SweGrepBackend({binary:config.binary,root,timeoutMs:30000,maxOutputBytes:2097152});
  const result=await backend.call(name,{...validated,path:target},{signal});
  return {...result,_swe:{node,hostname:os.hostname(),account:os.userInfo().username,path:target}};
}

export function createCodexServer(config){
  verifyIdentity(config);
  const lifecycle=new AbortController();
  const server=new Server({name:'swe-grep',version:'0.2.2'},{capabilities:{tools:{listChanged:false}},instructions:'Use SWE-Grep by default for code search and navigation. Before work, follow the user node-confirmation policy. Supply the already-confirmed target as node on every MCP call, including after SSH hops, and an absolute path on that node. Any readable local or mounted path is supported. Never change nodes on failure. Shell adapters, when installed, default to their own host. No sudo or credential changes are provided.'});
  const close=server.close.bind(server);
  server.close=async()=>{lifecycle.abort();await close();};
  server.onclose=()=>lifecycle.abort();
  server.setRequestHandler(ListToolsRequestSchema,async()=>({tools:toolContracts(config)}));
  server.setRequestHandler(CallToolRequestSchema,async(request,extra)=>{
    try{
      const result=await executeTool(config,request.params.name,request.params.arguments,{signal:AbortSignal.any([extra.signal,lifecycle.signal])});
      return {structuredContent:result,content:[{type:'text',text:JSON.stringify(result)}]};
    }catch(error){
      const result={error:{code:error.code??'SWE_GREP_ERROR',message:error.message}};
      return {isError:true,structuredContent:result,content:[{type:'text',text:JSON.stringify(result)}]};
    }
  });
  return server;
}

export async function serve(config){
  const server=createCodexServer(config);
  const shutdown=()=>{void server.close().catch(()=>{process.exitCode=1;});};
  process.once('SIGTERM',shutdown);process.once('SIGINT',shutdown);process.stdin.once('end',shutdown);process.stdin.once('close',shutdown);
  await server.connect(new StdioServerTransport());
}
