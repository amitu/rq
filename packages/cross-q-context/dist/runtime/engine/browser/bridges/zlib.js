import { gunzipSync, gzipSync, unzlibSync, zlibSync } from 'fflate';
export function browserZlibHandler(req) {
    switch (req.op) {
        case 'gzip':
            return { bytes: gzipSync(req.data) };
        case 'gunzip':
            return { bytes: gunzipSync(req.data) };
        // zlib-wrapped, matching Node's deflateSync/inflateSync — see the table above.
        case 'deflate':
            return { bytes: zlibSync(req.data) };
        case 'inflate':
            return { bytes: unzlibSync(req.data) };
    }
}
