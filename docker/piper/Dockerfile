# Local Piper text-to-speech HTTP server.
#
# Base image carries the Python runtime; the voice itself is downloaded at
# first start into the ./voices volume, so rebuilding the image does not
# re-download the model.

FROM python:3.13-slim

WORKDIR /voices

RUN pip install --no-cache-dir "piper-tts[http]"
