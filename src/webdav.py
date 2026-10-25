"""Serve work/share over WebDAV on 0.0.0.0:8088 (guest sees it at http://10.0.2.2:8088/)."""
from wsgidav.wsgidav_app import WsgiDAVApp
from cheroot import wsgi
app = WsgiDAVApp({"provider_mapping": {"/": "work/share"}, "simple_dc": {"user_mapping": {"*": True}},
                  "verbose": 1, "logging": {"enable_loggers": []}, "property_manager": True, "lock_storage": True})
srv = wsgi.Server(("0.0.0.0", 8088), app, numthreads=16)
print("webdav on :8088", flush=True)
srv.start()
