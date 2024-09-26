#include <climits>

#include <cstring>
#include <fstream>
#include <iostream>
#include <ostream>
#include <vector>
#include <wels/codec_api.h>
#include <wels/codec_app_def.h>
#include <wels/codec_def.h>
#include <wels/codec_ver.h>

void decodeNalUnits(const char *filename) {
  // Initialize the decoder
  ISVCDecoder *decoder;
  WelsCreateDecoder(&decoder);

  // Set up decoder parameters
  SDecodingParam decodingParams;
  memset(&decodingParams, 0, sizeof(SDecodingParam));
  decodingParams.uiTargetDqLayer = UCHAR_MAX;
  decodingParams.eEcActiveIdc = ERROR_CON_SLICE_COPY;
  decodingParams.sVideoProperty.eVideoBsType = VIDEO_BITSTREAM_DEFAULT;

  // Initialize decoder with the parameters
  if (decoder->Initialize(&decodingParams) != cmResultSuccess) {
    std::cerr << "Failed to initialize OpenH264 decoder" << std::endl;
    return;
  }

  // Open the file containing the NAL units
  std::ifstream file(filename, std::ios::binary | std::ios::ate);
  if (!file.is_open()) {
    std::cerr << "Failed to open file: " << filename << std::endl;
    return;
  }

  // Get the size of the file
  std::streamsize size = file.tellg();
  file.seekg(0, std::ios::beg);

  // Read the entire file into memory
  std::vector<unsigned char> buffer(size);
  if (!file.read(reinterpret_cast<char *>(buffer.data()), size)) {
    std::cerr << "Failed to read file: " << filename << std::endl;
    return;
  }

  // Decode the NAL units
  unsigned char *data[3] = {0}; // Output picture buffers
  SBufferInfo bufferInfo;
  memset(&bufferInfo, 0, sizeof(SBufferInfo));

  int nalIndex = 0;
  while (nalIndex < buffer.size()) {
    unsigned char *nalData = buffer.data() + nalIndex;

    int sliceSize = buffer.size() - nalIndex;
    DECODING_STATE state =
        decoder->DecodeFrameNoDelay(nalData, sliceSize, data, &bufferInfo);

    if (state != dsErrorFree) {
      std::cerr << "Error decoding frame. State: " << state << std::endl;
    } else if (bufferInfo.iBufferStatus == 1) {
      std::cout << "Frame decoded. Width: "
                << bufferInfo.UsrData.sSystemBuffer.iWidth
                << ", Height: " << bufferInfo.UsrData.sSystemBuffer.iHeight

                << std::endl;

    } else {
      std::cout << ":)" << std::endl;
    }

    nalIndex += sliceSize; // Move to next NAL unit (in a real case, find NAL
                           // boundaries properly)
  }

  // Clean up
  decoder->Uninitialize();
  WelsDestroyDecoder(decoder);
}

int main() {
  const char *filename =
      "test.h264"; // Replace with your file containing the NAL units
  decodeNalUnits(filename);
  return 0;
}
