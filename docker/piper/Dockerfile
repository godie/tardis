# Local Piper text-to-speech HTTP server.
#
# Base image carries the Python runtime; the voice itself is downloaded at
# first start into the ./voices volume, so rebuilding the image does not
# re-download the model.
#
# The base image digest and Piper version are pinned so the same checkout
# always builds the same server. See README section 8 before changing them.

FROM python:3.13-slim@sha256:e48f59cdba69a3cbd9ed2cf0f348d90d45a8d350d98134cf1070bc106ee9ed90

WORKDIR /voices

RUN pip install --no-cache-dir "piper-tts[http]==1.8.0"
