// SPDX-License-Identifier: GPL-3.0-or-later
import {execFileSync} from 'node:child_process';
// Child process failures may carry stdout, stderr, argv and raw provider responses.
// Do not serialize them. Only an allowlisted probe error crosses this boundary.
export function readOnlyExec(file,args,options={},execute=execFileSync){
 try{return execute(file,args,{timeout:60000,...options,stdio:['pipe','pipe','pipe']});}
 catch{throw new Error('PR_M0_READ_FAILED');}
}
