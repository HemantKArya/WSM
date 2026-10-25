"""Serve work/share over SMB2 on 0.0.0.0:4445 as share HOST (guest/anonymous). Guest: mount_smbfs //guest@10.0.2.2:4445/HOST"""
import os
from impacket import smbserver
srv = smbserver.SimpleSMBServer(listenAddress="0.0.0.0", listenPort=4445)
srv.addShare("HOST", os.path.abspath("work/share"), "wsm host share", readOnly="no")
srv.setSMB2Support(True)
srv.setSMBChallenge("")
srv.setLogFile("logs/X6/smb-server.log")
print("smb on :4445", flush=True)
srv.start()
